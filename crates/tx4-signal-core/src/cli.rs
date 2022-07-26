//! Server-side connection types.

use crate::*;
use futures::sink::SinkExt;
use futures::stream::StreamExt;
use lair_keystore_api::prelude::*;
use std::sync::Arc;
use tokio_tungstenite::tungstenite::Message;

type Socket = tokio_tungstenite::WebSocketStream<tokio_rustls::TlsStream<tokio::net::TcpStream>>;

/// A message received from the remote.
#[derive(Debug)]
pub enum SigMessage {
    /// An incoming webrtc "offer".
    Offer {
        /// Remote signal id.
        rem_id: Arc<Id>,
        /// Remote x25519 public key.
        rem_pk: Arc<Id>,
        /// The webrtc "offer".
        offer: serde_json::Value,
    },

    /// An incoming webrtc "answer".
    Answer {
        /// Remote signal id.
        rem_id: Arc<Id>,
        /// Remote x25519 public key.
        rem_pk: Arc<Id>,
        /// The webrtc "answer".
        answer: serde_json::Value,
    },

    /// An incoming webrtc ICE candidate.
    ICE {
        /// Remote signal id.
        rem_id: Arc<Id>,
        /// Remote x25519 public key.
        rem_pk: Arc<Id>,
        /// The webrtc "answer".
        ice: serde_json::Value,
    },

    /// An incoming demo broadcast.
    Demo {
        /// Remote signal id.
        rem_id: Arc<Id>,
        /// Remote x25519 public key.
        rem_pk: Arc<Id>,
    },
}

type RecvCb = Box<dyn FnMut(SigMessage) + 'static + Send>;

/// Builder for constructing a Cli instance.
pub struct CliBuilder {
    tls: tls::TlsConfig,
    recv_cb: RecvCb,
    lair_client: Option<LairClient>,
    lair_tag: Option<Arc<str>>,
    url: Option<url::Url>,
}

impl Default for CliBuilder {
    fn default() -> Self {
        let tls = tls::TlsConfigBuilder::default().build().unwrap();
        Self {
            tls,
            recv_cb: Box::new(|_| {}),
            lair_client: None,
            lair_tag: None,
            url: None,
        }
    }
}

impl CliBuilder {
    /// Set the TlsConfig.
    pub fn set_tls(&mut self, tls: tls::TlsConfig) {
        self.tls = tls;
    }

    /// Apply a TlsConfig.
    pub fn with_tls(mut self, tls: tls::TlsConfig) -> Self {
        self.set_tls(tls);
        self
    }

    /// Set the receiver callback.
    pub fn set_recv_cb<Cb>(&mut self, cb: Cb)
    where
        Cb: FnMut(SigMessage) + 'static + Send,
    {
        self.recv_cb = Box::new(cb);
    }

    /// Apply the receiver callback.
    pub fn with_recv_cb<Cb>(mut self, cb: Cb) -> Self
    where
        Cb: FnMut(SigMessage) + 'static + Send,
    {
        self.set_recv_cb(cb);
        self
    }

    /// Set the LairClient.
    pub fn set_lair_client(&mut self, lair_client: LairClient) {
        self.lair_client = Some(lair_client);
    }

    /// Apply the LairClient.
    pub fn with_lair_client(mut self, lair_client: LairClient) -> Self {
        self.set_lair_client(lair_client);
        self
    }

    /// Set the Lair tag.
    pub fn set_lair_tag(&mut self, lair_tag: Arc<str>) {
        self.lair_tag = Some(lair_tag);
    }

    /// Apply the Lair tag.
    pub fn with_lair_tag(mut self, lair_tag: Arc<str>) -> Self {
        self.set_lair_tag(lair_tag);
        self
    }

    /// Set the server url.
    pub fn set_url(&mut self, url: url::Url) {
        self.url = Some(url);
    }

    /// Apply the server url.
    pub fn with_url(mut self, url: url::Url) -> Self {
        self.set_url(url);
        self
    }

    /// Build the Srv instance.
    pub async fn build(self) -> Result<Cli> {
        Cli::priv_build(self).await
    }
}

const OFFER: u8 = 1;
const ANSWER: u8 = 2;
const ICE: u8 = 3;

/// Server-side connection type.
pub struct Cli {
    con_term: util::Term,
    addr: url::Url,
    ice_servers: serde_json::Value,
    sink: futures::stream::SplitSink<Socket, Message>,
    loc_pk: Arc<Id>,
    lair_client: LairClient,
}

impl Drop for Cli {
    fn drop(&mut self) {
        self.con_term.term();
    }
}

impl Cli {
    /// Get a CliBuilder.
    pub fn builder() -> CliBuilder {
        CliBuilder::default()
    }

    /// Shutdown this client instance.
    pub fn close(&self) {
        self.con_term.term();
    }

    /// Get the addr this cli can be reached at through the signal server.
    pub fn local_addr(&self) -> &url::Url {
        &self.addr
    }

    /// Get the ice server list provided by the server.
    pub fn ice_servers(&self) -> &serde_json::Value {
        &self.ice_servers
    }

    /// Make a webrtc offer to a remote peer.
    pub async fn offer<S>(&mut self, rem_id: &Id, rem_pk: &Id, offer: &S) -> Result<()>
    where
        S: ?Sized + serde::Serialize,
    {
        let offer = serde_json::to_string(offer)?;
        self.send(rem_id, rem_pk, OFFER, offer.as_bytes()).await
    }

    /// Send a webrtc answer to a remote peer.
    pub async fn answer<S>(&mut self, rem_id: &Id, rem_pk: &Id, answer: &S) -> Result<()>
    where
        S: ?Sized + serde::Serialize,
    {
        let answer = serde_json::to_string(answer)?;
        self.send(rem_id, rem_pk, ANSWER, answer.as_bytes()).await
    }

    /// Send a webrtc ice candidate to a remote peer.
    pub async fn ice<S>(&mut self, rem_id: &Id, rem_pk: &Id, ice: &S) -> Result<()>
    where
        S: ?Sized + serde::Serialize,
    {
        let ice = serde_json::to_string(ice)?;
        self.send(rem_id, rem_pk, ICE, ice.as_bytes()).await
    }

    /// Send a demo broadcast message to the server.
    /// (If server doesn't allow demo mode, this could get you banned).
    pub async fn demo(&mut self) -> Result<()> {
        let mut out = Vec::with_capacity(DEMO.len() + 32);
        out.extend_from_slice(DEMO);
        out.extend_from_slice(&*self.loc_pk);

        self.sink
            .send(Message::Binary(out))
            .await
            .map_err(other_err)
    }

    // -- private -- //

    async fn send(&mut self, rem_id: &Id, rem_pk: &Id, kind: u8, data: &[u8]) -> Result<()> {
        let mut msg = Vec::with_capacity(1 + data.len());
        msg.push(kind);
        msg.extend_from_slice(data);

        let (nonce, cipher) = self
            .lair_client
            .crypto_box_xsalsa_by_pub_key((&*self.loc_pk).into(), rem_pk.into(), None, msg.into())
            .await?;

        let mut out = Vec::with_capacity(FORWARD.len() + 32 + 32 + nonce.len() + cipher.len());
        out.extend_from_slice(FORWARD);
        out.extend_from_slice(rem_id);
        out.extend_from_slice(&*self.loc_pk);
        out.extend_from_slice(&nonce[..]);
        out.extend_from_slice(&*cipher);

        self.sink
            .send(Message::Binary(out))
            .await
            .map_err(other_err)
    }

    async fn priv_build(builder: CliBuilder) -> Result<Self> {
        let CliBuilder {
            tls,
            recv_cb,
            lair_client,
            lair_tag,
            url,
        } = builder;

        let lair_client = match lair_client {
            Some(lair_client) => lair_client,
            None => return Err(other_err("LairClientRequired")),
        };

        let lair_tag = match lair_tag {
            Some(lair_tag) => lair_tag,
            None => return Err(other_err("LairTagRequired")),
        };

        let mut url = match url {
            Some(url) => url,
            None => return Err(other_err("UrlRequired")),
        };

        let x25519_pub = match lair_client.get_entry(lair_tag).await {
            Ok(LairEntryInfo::Seed { tag: _, seed_info }) => {
                Id::from_slice(&*seed_info.x25519_pub_key.0)?
            }
            _ => return Err(other_err("lair_tag invalid seed")),
        };

        tracing::debug!(?x25519_pub);

        if url.scheme() != "hc-rtc-sig" {
            return Err(other_err(format!(
                "invalid scheme, expected \"hc-rtc-sig\", got {:?}",
                url.scheme()
            )));
        }

        let mut path_iter = url.path().split('/');

        let cert_digest = match path_iter.next() {
            Some(digest_b64) => match base64::decode_config(digest_b64, base64::URL_SAFE_NO_PAD) {
                Ok(digest) => {
                    if digest.len() != 32 {
                        return Err(other_err("InvalidUrlCertDigestLen"));
                    }
                    digest
                }
                Err(e) => return Err(other_err(e)),
            },
            None => return Err(other_err("InvalidUrlCertDigest")),
        };

        let mut err_list = Vec::new();
        let mut result_socket = None;

        'connect_loop: for addr in path_iter {
            for addr in tokio::net::lookup_host(addr).await? {
                tracing::debug!(?addr, "try connect");

                let socket = match tokio::net::TcpStream::connect(addr).await {
                    Ok(socket) => socket,
                    Err(err) => {
                        err_list.push(err);
                        continue;
                    }
                };

                let socket = match tcp_configure(socket) {
                    Ok(socket) => socket,
                    Err(err) => {
                        err_list.push(err);
                        continue;
                    }
                };

                let name = "stub".try_into().unwrap();

                let socket: tokio_rustls::TlsStream<tokio::net::TcpStream> =
                    match tokio_rustls::TlsConnector::from(tls.cli.clone())
                        .connect(name, socket)
                        .await
                    {
                        Ok(socket) => socket.into(),
                        Err(err) => {
                            err_list.push(err);
                            continue;
                        }
                    };

                let remote_id = match hash_cert(&socket) {
                    Ok(remote_id) => remote_id,
                    Err(err) => {
                        err_list.push(err);
                        continue;
                    }
                };

                if **remote_id != cert_digest {
                    err_list.push(other_err("InvalidRemoteCert"));
                    continue;
                }

                let (socket, _rsp) = match tokio_tungstenite::client_async_with_config(
                    "wss://stub",
                    socket,
                    Some(WS_CONFIG),
                )
                .await
                .map_err(other_err)
                {
                    Ok(r) => r,
                    Err(err) => {
                        err_list.push(err);
                        continue;
                    }
                };

                result_socket = Some(socket);
                break 'connect_loop;
            }
        }

        let socket = match result_socket {
            Some(socket) => socket,
            None => return Err(other_err(format!("{:?}", err_list))),
        };

        let (sink, mut stream) = socket.split();

        let (id, ice_servers) = match stream.next().await {
            Some(Ok(Message::Binary(data))) => {
                if &data[0..HELLO.len()] != HELLO {
                    return Err(other_err("InvalidHello"));
                }
                let id = Id::from_slice(&data[4..36])?;
                let ice: serde_json::Value = serde_json::from_slice(&data[36..])?;
                tracing::debug!(?id, %ice);
                (id, ice)
            }
            _ => return Err(other_err("InvalidHello")),
        };

        let con_term = util::Term::new("con_term", None);

        con_term.spawn_err(
            con_recv_task(stream, lair_client.clone(), x25519_pub.clone(), recv_cb),
            |err| {
                tracing::debug!("ConRecvError: {:?}", err);
            },
        );

        url.query_pairs_mut()
            .clear()
            .append_pair("i", &id.to_b64())
            .append_pair("x", &x25519_pub.to_b64());

        tracing::debug!(%url);

        Ok(Self {
            con_term,
            addr: url,
            ice_servers,
            sink,
            loc_pk: x25519_pub,
            lair_client,
        })
    }
}

async fn con_recv_task(
    mut stream: futures::stream::SplitStream<Socket>,
    lair_client: LairClient,
    x25519_pub: Arc<Id>,
    mut recv_cb: RecvCb,
) -> Result<()> {
    while let Some(msg) = stream.next().await {
        let bin_data: Vec<u8> = match msg.map_err(other_err)? {
            Message::Text(data) => data.into_bytes(),
            Message::Binary(data) => data,
            Message::Ping(data) => data,
            Message::Pong(data) => data,
            Message::Close(close) => {
                return Err(other_err(format!("{:?}", close)));
            }
            Message::Frame(_) => return Err(other_err("RawFrame")),
        };

        if bin_data.len() == 4 + 32 + 32 && &bin_data[0..4] == DEMO {
            let rem_id = Id::from_slice(&bin_data[4..36])?;
            let rem_pk = Id::from_slice(&bin_data[36..68])?;
            recv_cb(SigMessage::Demo { rem_id, rem_pk });
            continue;
        }

        if bin_data.len()
            < FORWARD.len()
                + 32
                + 32
                + 24
                + sodoken::crypto_box::curve25519xsalsa20poly1305::MACBYTES
        {
            return Err(other_err("InvalidMsg"));
        }

        if &bin_data[0..4] != FORWARD {
            return Err(other_err("InvalidMsg"));
        }

        let rem_id = Id::from_slice(&bin_data[4..36])?;
        let rem_pk = Id::from_slice(&bin_data[36..68])?;
        let mut nonce = [0; 24];
        nonce.copy_from_slice(&bin_data[68..92]);
        let cipher = bin_data[92..].to_vec().into();

        let msg = lair_client
            .crypto_box_xsalsa_open_by_pub_key(
                (&*rem_pk).into(),
                (&*x25519_pub).into(),
                None,
                nonce,
                cipher,
            )
            .await?;
        match msg[0] {
            OFFER => {
                let offer: serde_json::Value = serde_json::from_slice(&msg[1..])?;
                let offer = SigMessage::Offer {
                    rem_id,
                    rem_pk,
                    offer,
                };
                recv_cb(offer);
            }
            ANSWER => {
                let answer: serde_json::Value = serde_json::from_slice(&msg[1..])?;
                let answer = SigMessage::Answer {
                    rem_id,
                    rem_pk,
                    answer,
                };
                recv_cb(answer);
            }
            ICE => {
                let ice: serde_json::Value = serde_json::from_slice(&msg[1..])?;
                let ice = SigMessage::ICE {
                    rem_id,
                    rem_pk,
                    ice,
                };
                recv_cb(ice);
            }
            _ => return Err(other_err("InvalidMsgKind")),
        }
    }

    // always error on end so our term is called
    Err(other_err("ConClose"))
}

fn hash_cert(socket: &tokio_rustls::TlsStream<tokio::net::TcpStream>) -> Result<Arc<Id>> {
    let (_, c) = socket.get_ref();
    if let Some(chain) = c.peer_certificates() {
        if !chain.is_empty() {
            use sha2::Digest;
            let mut digest = sha2::Sha256::new();
            digest.update(&chain[0].0);
            let digest = Arc::new(Id(digest.finalize().into()));
            return Ok(digest);
        }
    }
    Err(other_err("InvalidPeerCert"))
}
