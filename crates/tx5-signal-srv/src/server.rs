use crate::deps::*;
use crate::*;
use futures::sink::SinkExt;
use futures::stream::StreamExt;
use sodoken::crypto_box::curve25519xsalsa20poly1305 as crypto_box;
use std::collections::HashMap;
use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;
use tx5_core::wire;

struct IntGaugeGuard(prometheus::IntGauge);

impl Drop for IntGaugeGuard {
    fn drop(&mut self) {
        self.0.dec();
    }
}

impl IntGaugeGuard {
    pub fn new(gauge: prometheus::IntGauge) -> Self {
        gauge.inc();
        Self(gauge)
    }
}

/// [exec_tx5_signal_srv] will return this driver future.
pub type ServerDriver =
    std::pin::Pin<Box<dyn Future<Output = ()> + 'static + Send>>;

/// The main entrypoint tx5-signal-server logic task.
pub fn exec_tx5_signal_srv(
    config: Config,
) -> Result<(SocketAddr, ServerDriver)> {
    // make sure our metrics are initialized
    let _ = &*METRICS_REQ_COUNT;
    let _ = &*METRICS_REQ_TIME_S;
    let _ = &*CLIENT_ACTIVE_WS_COUNT;
    let _ = &*CLIENT_WS_COUNT;
    let _ = &*CLIENT_AUTH_WS_COUNT;
    let _ = &*CLIENT_WS_REQ_TIME_S;
    let _ = &*REQ_FWD_CNT;
    let _ = &*REQ_DEMO_CNT;

    let ice_servers = Arc::new(config.ice_servers);
    let srv_hnd = Srv::spawn();

    use warp::Filter;
    use warp::Reply;

    let tx5_ws = warp::path!("tx5-ws" / String)
        .and(warp::ws())
        .map(move |client_pub: String, ws: warp::ws::Ws| {
            let active_ws_g =
                IntGaugeGuard::new(CLIENT_ACTIVE_WS_COUNT.clone());
            CLIENT_WS_COUNT.inc();
            let client_pub = match decode_client_pub(&client_pub) {
                Err(err) => return reply_err(err).into_response(),
                Ok(client_pub) => client_pub,
            };
            let ice_servers = ice_servers.clone();
            let srv_hnd = srv_hnd.clone();
            ws.max_send_queue(tx5_core::ws::MAX_SEND_QUEUE)
                .max_message_size(tx5_core::ws::MAX_MESSAGE_SIZE)
                .max_frame_size(tx5_core::ws::MAX_FRAME_SIZE)
                .on_upgrade(move |ws| async move {
                    client_task(ws, client_pub, ice_servers, srv_hnd).await;
                    drop(active_ws_g);
                })
                .into_response()
        })
        .with(warp::trace::named("tx5-ws"));

    let prometheus = warp::path!("metrics")
        .map(move || {
            METRICS_REQ_COUNT.inc();
            let _time_g = METRICS_REQ_TIME_S.start_timer();

            let enc = prometheus::TextEncoder::new();
            let metrics = prometheus::default_registry().gather();
            enc.encode_to_string(&metrics).unwrap()
        })
        .with(warp::trace::named("metrics"));

    let routes = tx5_ws.or(prometheus).with(warp::trace::request());

    warp::serve(routes)
        .try_bind_ephemeral(([0, 0, 0, 0], config.port))
        .map_err(Error::err)
        .map(|(addr, fut)| {
            let fut: ServerDriver = Box::pin(fut);
            (addr, fut)
        })
}

fn decode_client_pub(
    c: &str,
) -> Result<sodoken::BufReadSized<{ crypto_box::PUBLICKEYBYTES }>> {
    let c = base64::decode_config(c.as_bytes(), base64::URL_SAFE_NO_PAD)
        .map_err(Error::err)?;
    if c.len() != crypto_box::PUBLICKEYBYTES {
        return Err(Error::id("InvalidClientPubKey"));
    }
    let client_pub = sodoken::BufWriteSized::new_no_lock();
    client_pub.write_lock().copy_from_slice(&c);
    Ok(client_pub.to_read_sized())
}

fn reply_err(err: std::io::Error) -> impl warp::reply::Reply {
    let err = err.to_string();
    warp::reply::with_status(
        warp::reply::html(format!(
            r#"<!DOCTYPE html>
<html>
    <head>{err}</head>
    <body>
        <h1>{err}</h1>
    </body>
</html>
"#,
        )),
        warp::http::StatusCode::BAD_REQUEST,
    )
}

async fn client_task(
    mut ws: warp::ws::WebSocket,
    client_pub: sodoken::BufReadSized<{ crypto_box::PUBLICKEYBYTES }>,
    ice_servers: Arc<serde_json::Value>,
    srv_hnd: Arc<SrvHnd>,
) {
    macro_rules! dbg_err {
        ($e:expr) => {
            match $e {
                Err(err) => {
                    tracing::debug!(?err);
                    return;
                }
                Ok(r) => r,
            }
        };
    }

    let client_id = dbg_err!(Id::from_slice(&client_pub.read_lock()));

    dbg_err!(authenticate(&mut ws, client_pub, ice_servers).await);

    let (mut tx, mut rx) = ws.split();
    let (out_send, mut out_recv) = tokio::sync::mpsc::unbounded_channel();

    dbg_err!(srv_hnd.register(client_id, out_send.clone()).await);

    CLIENT_AUTH_WS_COUNT.inc();

    let srv_hnd_read = srv_hnd.clone();
    let client_id_read = client_id;
    tokio::select! {
        res = async move {
            while let Some((msg, resp)) = out_recv.recv().await {
                let len = msg.as_bytes().len();
                if let Err(err) = tx.send(msg).await {
                    let err = Error::err(err);
                    tracing::debug!(?err);
                    let _ = resp.send(Err(err.err_clone()));
                    return;
                } else {
                    let _ = resp.send(Ok(()));
                }
                tracing::trace!("ws send {} bytes", len);
            }
        } => res,

        res = async move {
            while let Some(msg) = rx.next().await {
                let _time_g = CLIENT_WS_REQ_TIME_S.start_timer();

                let msg = match msg {
                    Err(err) => {
                        tracing::debug!(?err);
                        break;
                    }
                    Ok(msg) => msg,
                };

                if msg.is_pong() {
                    tracing::trace!("got pong");
                    continue;
                }

                if msg.is_ping() {
                    tracing::trace!("got ping");
                    // warp handles responding with a pong automagically
                    continue;
                }

                let msg = msg.into_bytes();
                tracing::trace!("ws recv {} bytes", msg.len());

                match wire::Wire::decode(&msg) {
                    Err(err) => {
                        tracing::debug!(?err);
                        return;
                    }
                    Ok(wire::Wire::DemoV1 {
                        rem_pub: _,
                    }) => {
                        // TODO - pay attention to demo config flag
                        //        right now we just always honor demos
                        srv_hnd_read.broadcast(msg);
                        REQ_DEMO_CNT.inc();
                    }
                    Ok(wire::Wire::FwdV1 { rem_pub, nonce, cipher }) => {
                        let data = dbg_err!(wire::Wire::FwdV1 {
                            rem_pub: client_id_read,
                            nonce,
                            cipher,
                        }.encode());
                        match srv_hnd_read.forward(rem_pub, data).await {
                            Ok(fut) => {
                                match fut.await {
                                    Ok(Ok(())) => (),
                                    Ok(Err(err)) => {
                                        tracing::trace!(?err);
                                    }
                                    Err(_) => (),
                                }
                            }
                            Err(err) => {
                                tracing::trace!(?err);
                            }
                        }
                        REQ_FWD_CNT.inc();
                    }
                    _ => {
                        tracing::debug!("InvalidClientMsg");
                        return;
                    }
                }
            }
        } => res,
    };

    tracing::debug!("ConShutdown");
    dbg_err!(srv_hnd.unregister(client_id).await);
}

async fn authenticate(
    ws: &mut warp::ws::WebSocket,
    client_pub: sodoken::BufReadSized<{ crypto_box::PUBLICKEYBYTES }>,
    ice_servers: Arc<serde_json::Value>,
) -> Result<()> {
    let src_con_key = <sodoken::BufWriteSized<32>>::new_no_lock();
    sodoken::random::bytes_buf(src_con_key.clone()).await?;
    let src_con_key = src_con_key.to_read_sized();

    let srv_pubkey = sodoken::BufWriteSized::new_no_lock();
    let srv_seckey = sodoken::BufWriteSized::new_mem_locked()?;
    crypto_box::keypair(srv_pubkey.clone(), srv_seckey.clone()).await?;

    let nonce = sodoken::BufWriteSized::new_no_lock();
    sodoken::random::bytes_buf(nonce.clone()).await?;

    let cipher = crypto_box::easy(
        nonce.clone(),
        src_con_key.clone(),
        client_pub.clone(),
        srv_seckey,
    )
    .await?;

    let auth_req = wire::Wire::AuthReqV1 {
        srv_pub: (*srv_pubkey.read_lock_sized()).into(),
        nonce: (*nonce.read_lock_sized()).into(),
        cipher: cipher.read_lock().to_vec().into_boxed_slice().into(),
        ice: (*ice_servers).clone(),
    }
    .encode()?;

    ws.send(warp::ws::Message::binary(auth_req))
        .await
        .map_err(Error::err)?;

    let resp = match ws.next().await {
        None => return Err(Error::id("ClosedWithoutAuthResp")),
        Some(Err(err)) => return Err(Error::err(err)),
        Some(Ok(resp)) => resp.into_bytes(),
    };

    match wire::Wire::decode(&resp) {
        Err(err) => return Err(err),
        Ok(wire::Wire::AuthResV1 {
            con_key,
            // TODO just registering addy for everyone right now...
            req_addr: _,
        }) => {
            if *con_key != *src_con_key.read_lock() {
                return Err(Error::id("InvalidConKey"));
            }
        }
        _ => return Err(Error::id("InvalidAuthResponse")),
    }

    Ok(())
}

type OneSend<T> = tokio::sync::oneshot::Sender<T>;
type OneRecv<T> = tokio::sync::oneshot::Receiver<T>;

type DataSend = tokio::sync::mpsc::UnboundedSender<(
    warp::ws::Message,
    OneSend<Result<()>>,
)>;

enum SrvCmd {
    Shutdown,
    Register(Id, DataSend, OneSend<Result<()>>),
    Unregister(Id, OneSend<Result<()>>),
    Forward(Id, Vec<u8>, OneSend<Result<OneRecv<Result<()>>>>),
    Broadcast(Vec<u8>),
}

type SrvSend = tokio::sync::mpsc::UnboundedSender<SrvCmd>;

struct SrvHnd(SrvSend, tokio::task::JoinHandle<()>);

impl Drop for SrvHnd {
    fn drop(&mut self) {
        self.shutdown();
    }
}

const E_SERVER_SHUTDOWN: &str = "ServerShutdown";

impl SrvHnd {
    pub fn shutdown(&self) {
        let _ = self.0.send(SrvCmd::Shutdown);
        self.1.abort();
    }

    pub async fn register(&self, id: Id, data_send: DataSend) -> Result<()> {
        let (s, r) = tokio::sync::oneshot::channel();
        if self.0.send(SrvCmd::Register(id, data_send, s)).is_err() {
            return Err(Error::id(E_SERVER_SHUTDOWN));
        }
        r.await.map_err(|_| Error::id(E_SERVER_SHUTDOWN))?
    }

    pub async fn unregister(&self, id: Id) -> Result<()> {
        let (s, r) = tokio::sync::oneshot::channel();
        if self.0.send(SrvCmd::Unregister(id, s)).is_err() {
            return Err(Error::id(E_SERVER_SHUTDOWN));
        }
        r.await.map_err(|_| Error::id(E_SERVER_SHUTDOWN))?
    }

    pub async fn forward(
        &self,
        id: Id,
        data: Vec<u8>,
    ) -> Result<OneRecv<Result<()>>> {
        let (s, r) = tokio::sync::oneshot::channel();
        if self.0.send(SrvCmd::Forward(id, data, s)).is_err() {
            return Err(Error::id(E_SERVER_SHUTDOWN));
        }
        r.await.map_err(|_| Error::id(E_SERVER_SHUTDOWN))?
    }

    pub fn broadcast(&self, data: Vec<u8>) {
        let _ = self.0.send(SrvCmd::Broadcast(data));
    }
}

struct Srv {
    cons: HashMap<Id, DataSend>,
}

impl Srv {
    fn new() -> Self {
        Self {
            cons: HashMap::new(),
        }
    }

    fn spawn() -> Arc<SrvHnd> {
        let (hnd_send, mut hnd_recv) = tokio::sync::mpsc::unbounded_channel();
        let mut srv = Srv::new();

        let task = tokio::task::spawn(async move {
            while let Some(cmd) = hnd_recv.recv().await {
                if !srv.sync_process(cmd) {
                    break;
                }
            }
        });

        Arc::new(SrvHnd(hnd_send, task))
    }

    // we want to make sure this function is *not* async
    // so that we can chew through the work loop without stalling
    fn sync_process(&mut self, cmd: SrvCmd) -> bool {
        match cmd {
            SrvCmd::Shutdown => return false,
            SrvCmd::Register(id, data_send, resp) => {
                let _ = resp.send(self.register(id, data_send));
            }
            SrvCmd::Unregister(id, resp) => {
                let _ = resp.send(self.unregister(id));
            }
            SrvCmd::Forward(id, data, resp) => {
                let _ = resp.send(self.forward(id, data));
            }
            SrvCmd::Broadcast(data) => {
                self.broadcast(data);
            }
        }
        true
    }

    fn register(&mut self, id: Id, data_send: DataSend) -> Result<()> {
        self.cons.insert(id, data_send);
        Ok(())
    }

    fn unregister(&mut self, id: Id) -> Result<()> {
        self.cons.remove(&id);
        Ok(())
    }

    fn forward(
        &mut self,
        id: Id,
        data: Vec<u8>,
    ) -> Result<OneRecv<Result<()>>> {
        if let Some(data_send) = self.cons.get(&id) {
            let (s, r) = tokio::sync::oneshot::channel();
            if data_send
                .send((warp::ws::Message::binary(data), s))
                .is_err()
            {
                self.cons.remove(&id);
            } else {
                return Ok(r);
            }
        }

        Err(Error::id("InvalidForwardTarget"))
    }

    fn broadcast(&mut self, data: Vec<u8>) {
        for (_, data_send) in self.cons.iter() {
            let (s, _) = tokio::sync::oneshot::channel();
            let _ =
                data_send.send((warp::ws::Message::binary(data.clone()), s));
        }
    }
}
