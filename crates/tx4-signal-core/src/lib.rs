#![deny(missing_docs)]
#![deny(warnings)]
#![deny(unsafe_code)]

//! Holochain webrtc signal server / client.
//!
//! [![Project](https://img.shields.io/badge/project-holochain-blue.svg?style=flat-square)](http://holochain.org/)
//! [![Forum](https://img.shields.io/badge/chat-forum%2eholochain%2enet-blue.svg?style=flat-square)](https://forum.holochain.org)
//! [![Chat](https://img.shields.io/badge/chat-chat%2eholochain%2enet-blue.svg?style=flat-square)](https://chat.holochain.org)
//!
//! [![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://opensource.org/licenses/MIT)
//! [![License: Apache-2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://www.apache.org/licenses/LICENSE-2.0)
//!

#![doc = include_str!("docs/srv_help.md")]

use std::io::Result;
use std::sync::Arc;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;

/// Hc-rtc-sig identifier.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Id(pub [u8; 32]);

impl std::fmt::Debug for Id {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut a = self.to_b64();
        a.replace_range(8..a.len() - 8, "..");
        f.write_str(&a)
    }
}

impl std::fmt::Display for Id {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_b64())
    }
}

impl std::ops::Deref for Id {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.0[..]
    }
}

impl AsRef<[u8]> for Id {
    fn as_ref(&self) -> &[u8] {
        &self.0[..]
    }
}

impl From<&Id> for lair_keystore_api::encoding_types::BinDataSized<32> {
    #[inline]
    fn from(id: &Id) -> Self {
        Self(Arc::new(id.0))
    }
}

impl From<Id> for lair_keystore_api::encoding_types::BinDataSized<32> {
    #[inline]
    fn from(id: Id) -> Self {
        (&id).into()
    }
}

impl Id {
    /// Load from a slice.
    pub fn from_slice(s: &[u8]) -> Result<Arc<Self>> {
        if s.len() != 32 {
            return Err(other_err("InvalidIdLength"));
        }
        let mut out = [0; 32];
        out.copy_from_slice(s);
        Ok(Arc::new(Self(out)))
    }

    /// Decode a base64 encoded Id.
    pub fn from_b64(s: &str) -> Result<Arc<Self>> {
        let v = base64::decode_config(s, base64::URL_SAFE_NO_PAD).map_err(other_err)?;
        Self::from_slice(&v)
    }

    /// Encode a Id as base64.
    pub fn to_b64(&self) -> String {
        base64::encode_config(self, base64::URL_SAFE_NO_PAD)
    }
}

const HELLO: &[u8] = b"hrsH";
const FORWARD: &[u8] = b"hrsF";
const DEMO: &[u8] = b"hrsD";

/// Tx3 helper until `std::io::Error::other()` is stablized
pub fn other_err<E: Into<Box<dyn std::error::Error + Send + Sync>>>(error: E) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Other, error)
}

/// Extract a signal id from an hc-rtc-sig client address url.
pub fn signal_id_from_addr(addr: &url::Url) -> Result<Arc<Id>> {
    for (k, v) in addr.query_pairs() {
        if k == "i" {
            return Id::from_b64(&v);
        }
    }
    Err(other_err("InvalidUrl"))
}

/// Extract an x25519 pk from an hc-rtc-sig client address url.
pub fn pk_from_addr(addr: &url::Url) -> Result<Arc<Id>> {
    for (k, v) in addr.query_pairs() {
        if k == "x" {
            return Id::from_b64(&v);
        }
    }
    Err(other_err("InvalidUrl"))
}

pub(crate) static WS_CONFIG: WebSocketConfig = WebSocketConfig {
    max_send_queue: Some(32),
    max_message_size: Some(1024),
    max_frame_size: Some(1024),
    accept_unmasked_frames: false,
};

pub(crate) fn tcp_configure(socket: tokio::net::TcpStream) -> Result<tokio::net::TcpStream> {
    let socket = socket.into_std()?;
    let socket = socket2::Socket::from(socket);

    let keepalive = socket2::TcpKeepalive::new()
        .with_time(std::time::Duration::from_secs(7))
        .with_interval(std::time::Duration::from_secs(7));

    // we'll close unresponsive connections after 21-28 seconds (7 * 3)
    // (it's a little unclear how long it'll wait after the final probe)
    #[cfg(any(target_os = "linux", target_vendor = "apple"))]
    let keepalive = keepalive.with_retries(3);

    socket.set_tcp_keepalive(&keepalive)?;

    let socket = std::net::TcpStream::from(socket);
    tokio::net::TcpStream::from_std(socket)
}

pub mod cli;
pub mod srv;
pub mod tls;
pub mod util;

#[cfg(test)]
mod tests {
    use super::*;
    use lair_keystore_api::prelude::*;
    use std::sync::Arc;

    const ICE_SERVERS: &str = r#"[
        {
          "urls": ["stun:openrelay.metered.ca:80"]
        },
        {
          "urls": ["turn:openrelay.metered.ca:80"],
          "username": "openrelayproject",
          "credential": "openrelayproject"
        },
        {
          "urls": ["turn:openrelay.metered.ca:443"],
          "username": "openrelayproject",
          "credential": "openrelayproject"
        },
        {
          "urls": ["turn:openrelay.metered.ca:443?transport=tcp"],
          "username": "openrelayproject",
          "credential": "openrelayproject"
        }
    ]"#;

    fn init_tracing() {
        let subscriber = tracing_subscriber::FmtSubscriber::builder()
            .with_env_filter(tracing_subscriber::filter::EnvFilter::from_default_env())
            .with_file(true)
            .with_line_number(true)
            .finish();
        let _ = tracing::subscriber::set_global_default(subscriber);
    }

    struct Test {
        pub _keystore: lair_keystore_api::in_proc_keystore::InProcKeystore,
        pub cli: cli::Cli,
    }

    impl Test {
        pub async fn new<Cb>(srv_addr: url::Url, recv_cb: Cb) -> Self
        where
            Cb: FnMut(cli::SigMessage) + 'static + Send,
        {
            let passphrase = sodoken::BufRead::new_no_lock(b"test-passphrase");
            let keystore_config = PwHashLimits::Minimum
                .with_exec(|| LairServerConfigInner::new("/", passphrase.clone()))
                .await
                .unwrap();

            let keystore = PwHashLimits::Minimum
                .with_exec(|| {
                    lair_keystore_api::in_proc_keystore::InProcKeystore::new(
                        Arc::new(keystore_config),
                        lair_keystore_api::mem_store::create_mem_store_factory(),
                        passphrase,
                    )
                })
                .await
                .unwrap();

            let lair_client = keystore.new_client().await.unwrap();
            let tag: Arc<str> = rand_utf8::rand_utf8(&mut rand::thread_rng(), 32).into();

            lair_client
                .new_seed(tag.clone(), None, false)
                .await
                .unwrap();

            let cli = cli::Cli::builder()
                .with_lair_client(lair_client)
                .with_lair_tag(tag)
                .with_recv_cb(recv_cb)
                .with_url(srv_addr)
                .build()
                .await
                .unwrap();

            Self {
                _keystore: keystore,
                cli,
            }
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn sanity() {
        init_tracing();

        let (cert, key) = tls::gen_tls_cert_pair().unwrap();
        let tls = tls::TlsConfigBuilder::default()
            .with_cert(cert, key)
            .build()
            .unwrap();
        let srv = srv::Srv::builder()
            .with_tls(tls)
            .with_bind("127.0.0.1:0".parse().unwrap(), "127.0.0.1".into(), 0)
            .with_bind("[::1]:0".parse().unwrap(), "[::1]".into(), 0)
            .with_ice_servers(ICE_SERVERS.to_string())
            .with_allow_demo(true)
            .build()
            .await
            .unwrap();

        let srv_addr = srv.local_addr().clone();

        #[derive(Debug)]
        enum In {
            Cli1(cli::SigMessage),
            Cli2(cli::SigMessage),
        }

        let (in_send, mut in_recv) = tokio::sync::mpsc::unbounded_channel();

        let mut cli1 = {
            let in_send = in_send.clone();
            Test::new(srv_addr.clone(), move |msg| {
                in_send.send(In::Cli1(msg)).unwrap();
            })
            .await
        };
        let cli1_addr = cli1.cli.local_addr().clone();
        let cli1_sig_id = signal_id_from_addr(&cli1_addr).unwrap();
        let cli1_pk = pk_from_addr(&cli1_addr).unwrap();
        tracing::info!(?cli1_sig_id, ?cli1_pk);

        let mut cli2 = Test::new(srv_addr, move |msg| {
            in_send.send(In::Cli2(msg)).unwrap();
        })
        .await;
        let cli2_addr = cli2.cli.local_addr().clone();
        let cli2_sig_id = signal_id_from_addr(&cli2_addr).unwrap();
        let cli2_pk = pk_from_addr(&cli2_addr).unwrap();
        tracing::info!(?cli2_sig_id, ?cli2_pk);

        cli1.cli
            .offer(
                &cli2_sig_id,
                &cli2_pk,
                &serde_json::json!({ "type": "offer" }),
            )
            .await
            .unwrap();

        let msg = in_recv.recv().await;
        tracing::info!(?msg);
        assert!(matches!(msg, Some(In::Cli2(cli::SigMessage::Offer { .. }))));

        cli2.cli
            .answer(
                &cli1_sig_id,
                &cli1_pk,
                &serde_json::json!({ "type": "answer" }),
            )
            .await
            .unwrap();

        let msg = in_recv.recv().await;
        tracing::info!(?msg);
        assert!(matches!(
            msg,
            Some(In::Cli1(cli::SigMessage::Answer { .. }))
        ));

        cli1.cli
            .ice(
                &cli2_sig_id,
                &cli2_pk,
                &serde_json::json!({ "type": "ice" }),
            )
            .await
            .unwrap();

        let msg = in_recv.recv().await;
        tracing::info!(?msg);
        assert!(matches!(msg, Some(In::Cli2(cli::SigMessage::ICE { .. }))));

        cli1.cli.demo().await.unwrap();

        for _ in 0..2 {
            let msg = in_recv.recv().await;
            tracing::info!(?msg);
            let inner = match msg {
                Some(In::Cli1(m)) => m,
                Some(In::Cli2(m)) => m,
                _ => panic!("unexpected eos"),
            };
            assert!(matches!(inner, cli::SigMessage::Demo { .. }));
        }
    }
}
