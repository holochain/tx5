use super::*;
use std::sync::atomic::Ordering;

pub(crate) enum ConnCmd {
    SigRecv(tx5_signal::SignalMessage),
    WebrtcRecv(webrtc::WebrtcEvt),
    SendMessage(Vec<u8>),
    WebrtcTimeoutCheck,
    WebrtcClosed,
}

/// Receive messages from a tx5 connection.
pub struct ConnRecv(CloseRecv<Vec<u8>>);

impl ConnRecv {
    /// Receive up to 16KiB of message data.
    pub async fn recv(&mut self) -> Option<Vec<u8>> {
        self.0.recv().await
    }
}

/// A tx5 connection.
pub struct Conn {
    ready: Arc<tokio::sync::Semaphore>,
    pub_key: PubKey,
    cmd_send: CloseSend<ConnCmd>,
    conn_task: tokio::task::JoinHandle<()>,
    keepalive_task: tokio::task::JoinHandle<()>,
    is_webrtc: Arc<std::sync::atomic::AtomicBool>,
    send_msg_count: Arc<std::sync::atomic::AtomicU64>,
    send_byte_count: Arc<std::sync::atomic::AtomicU64>,
    recv_msg_count: Arc<std::sync::atomic::AtomicU64>,
    recv_byte_count: Arc<std::sync::atomic::AtomicU64>,
    hub_cmd_send: tokio::sync::mpsc::Sender<HubCmd>,
}

macro_rules! netaudit {
    ($lvl:ident, $($all:tt)*) => {
        ::tracing::event!(
            target: "NETAUDIT",
            ::tracing::Level::$lvl,
            m = "tx5-connection",
            $($all)*
        );
    };
}

impl Drop for Conn {
    fn drop(&mut self) {
        netaudit!(DEBUG, pub_key = ?self.pub_key, a = "drop");

        self.conn_task.abort();
        self.keepalive_task.abort();

        let hub_cmd_send = self.hub_cmd_send.clone();
        let pub_key = self.pub_key.clone();
        tokio::task::spawn(async move {
            let _ = hub_cmd_send.send(HubCmd::Disconnect(pub_key)).await;
        });
    }
}

impl Conn {
    #[cfg(test)]
    pub(crate) fn test_kill_keepalive_task(&self) {
        self.keepalive_task.abort();
    }

    pub(crate) fn priv_new(
        webrtc_config: Vec<u8>,
        is_polite: bool,
        pub_key: PubKey,
        client: Weak<tx5_signal::SignalConnection>,
        config: Arc<HubConfig>,
        hub_cmd_send: tokio::sync::mpsc::Sender<HubCmd>,
    ) -> (Arc<Self>, ConnRecv, CloseSend<ConnCmd>) {
        netaudit!(
            DEBUG,
            webrtc_config = String::from_utf8_lossy(&webrtc_config).to_string(),
            ?pub_key,
            ?is_polite,
            a = "open",
        );

        let is_webrtc = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let send_msg_count = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let send_byte_count = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let recv_msg_count = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let recv_byte_count = Arc::new(std::sync::atomic::AtomicU64::new(0));

        // zero len semaphore.. we actually just wait for the close
        let ready = Arc::new(tokio::sync::Semaphore::new(0));

        let (mut msg_send, msg_recv) = CloseSend::sized_channel(1024);
        let (cmd_send, cmd_recv) = CloseSend::sized_channel(1024);

        let keepalive_dur = config.signal_config.max_idle / 2;
        let client2 = client.clone();
        let pub_key2 = pub_key.clone();
        let keepalive_task = tokio::task::spawn(async move {
            loop {
                tokio::time::sleep(keepalive_dur).await;

                if let Some(client) = client2.upgrade() {
                    if client.send_keepalive(&pub_key2).await.is_err() {
                        break;
                    }
                } else {
                    break;
                }
            }
        });

        msg_send.set_close_on_drop(true);

        let con_task_fut = con_task(
            is_polite,
            webrtc_config,
            TaskCore {
                client,
                config,
                pub_key: pub_key.clone(),
                cmd_send: cmd_send.clone(),
                cmd_recv,
                send_msg_count: send_msg_count.clone(),
                send_byte_count: send_byte_count.clone(),
                recv_msg_count: recv_msg_count.clone(),
                recv_byte_count: recv_byte_count.clone(),
                msg_send,
                ready: ready.clone(),
                is_webrtc: is_webrtc.clone(),
            },
        );
        let conn_task = tokio::task::spawn(con_task_fut);

        let mut cmd_send2 = cmd_send.clone();
        cmd_send2.set_close_on_drop(true);
        let this = Self {
            ready,
            pub_key,
            cmd_send: cmd_send2,
            conn_task,
            keepalive_task,
            is_webrtc,
            send_msg_count,
            send_byte_count,
            recv_msg_count,
            recv_byte_count,
            hub_cmd_send,
        };

        (Arc::new(this), ConnRecv(msg_recv), cmd_send)
    }

    /// Wait until this connection is ready to send / receive data.
    pub async fn ready(&self) {
        // this will error when we close the semaphore waking up the task
        let _ = self.ready.acquire().await;
    }

    /// Returns `true` if we sucessfully connected over webrtc.
    pub fn is_using_webrtc(&self) -> bool {
        self.is_webrtc.load(Ordering::SeqCst)
    }

    /// The pub key of the remote peer this is connected to.
    pub fn pub_key(&self) -> &PubKey {
        &self.pub_key
    }

    /// Send up to 16KiB of message data.
    pub async fn send(&self, msg: Vec<u8>) -> Result<()> {
        self.cmd_send.send(ConnCmd::SendMessage(msg)).await
    }

    /// Get connection statistics.
    pub fn get_stats(&self) -> ConnStats {
        ConnStats {
            send_msg_count: self.send_msg_count.load(Ordering::Relaxed),
            send_byte_count: self.send_byte_count.load(Ordering::Relaxed),
            recv_msg_count: self.recv_msg_count.load(Ordering::Relaxed),
            recv_byte_count: self.recv_byte_count.load(Ordering::Relaxed),
        }
    }
}

/// Connection statistics.
#[derive(Default)]
pub struct ConnStats {
    /// message count sent.
    pub send_msg_count: u64,

    /// byte count sent.
    pub send_byte_count: u64,

    /// message count received.
    pub recv_msg_count: u64,

    /// byte count received.
    pub recv_byte_count: u64,
}

struct TaskCore {
    config: Arc<HubConfig>,
    client: Weak<tx5_signal::SignalConnection>,
    pub_key: PubKey,
    cmd_send: CloseSend<ConnCmd>,
    cmd_recv: CloseRecv<ConnCmd>,
    msg_send: CloseSend<Vec<u8>>,
    ready: Arc<tokio::sync::Semaphore>,
    is_webrtc: Arc<std::sync::atomic::AtomicBool>,
    send_msg_count: Arc<std::sync::atomic::AtomicU64>,
    send_byte_count: Arc<std::sync::atomic::AtomicU64>,
    recv_msg_count: Arc<std::sync::atomic::AtomicU64>,
    recv_byte_count: Arc<std::sync::atomic::AtomicU64>,
}

impl TaskCore {
    async fn handle_recv_msg(
        &self,
        msg: Vec<u8>,
    ) -> std::result::Result<(), ()> {
        self.recv_msg_count.fetch_add(1, Ordering::Relaxed);
        self.recv_byte_count
            .fetch_add(msg.len() as u64, Ordering::Relaxed);
        if self.msg_send.send(msg).await.is_err() {
            netaudit!(
                DEBUG,
                pub_key = ?self.pub_key,
                a = "close: msg_send closed",
            );
            Err(())
        } else {
            Ok(())
        }
    }

    fn track_send_msg(&self, len: usize) {
        self.send_msg_count.fetch_add(1, Ordering::Relaxed);
        self.send_byte_count
            .fetch_add(len as u64, Ordering::Relaxed);
    }
}

async fn con_task(
    is_polite: bool,
    webrtc_config: Vec<u8>,
    mut task_core: TaskCore,
) {
    // first process the handshake
    if let Some(client) = task_core.client.upgrade() {
        let handshake_fut = async {
            let nonce = client.send_handshake_req(&task_core.pub_key).await?;

            let mut got_peer_res = false;
            let mut sent_our_res = false;

            while let Some(cmd) = task_core.cmd_recv.recv().await {
                match cmd {
                    ConnCmd::SigRecv(sig) => {
                        use tx5_signal::SignalMessage::*;
                        match sig {
                            HandshakeReq(oth_nonce) => {
                                client
                                    .send_handshake_res(
                                        &task_core.pub_key,
                                        oth_nonce,
                                    )
                                    .await?;
                                sent_our_res = true;
                            }
                            HandshakeRes(res_nonce) => {
                                if res_nonce != nonce {
                                    return Err(Error::other("nonce mismatch"));
                                }
                                got_peer_res = true;
                            }
                            // Ignore all other message types...
                            // they may be from previous sessions
                            _ => (),
                        }
                    }
                    ConnCmd::SendMessage(_) => {
                        return Err(Error::other("send before ready"));
                    }
                    ConnCmd::WebrtcTimeoutCheck
                    | ConnCmd::WebrtcRecv(_)
                    | ConnCmd::WebrtcClosed => {
                        // only emitted by the webrtc module
                        // which at this point hasn't yet been initialized
                        unreachable!()
                    }
                }
                if got_peer_res && sent_our_res {
                    break;
                }
            }

            Result::Ok(())
        };

        match tokio::time::timeout(
            task_core.config.signal_config.max_idle,
            handshake_fut,
        )
        .await
        {
            Err(_) | Ok(Err(_)) => {
                client.close_peer(&task_core.pub_key).await;
                return;
            }
            Ok(Ok(_)) => (),
        }
    } else {
        return;
    }

    // next, attempt webrtc
    let task_core = match con_task_attempt_webrtc(
        is_polite,
        webrtc_config,
        task_core,
    )
    .await
    {
        AttemptWebrtcResult::Abort => return,
        AttemptWebrtcResult::Fallback(task_core) => task_core,
    };

    task_core.is_webrtc.store(false, Ordering::SeqCst);

    // if webrtc failed in a way that allows us to fall back to sbd,
    // use the fallback sbd messaging system
    con_task_fallback_use_signal(task_core).await;
}

async fn recv_cmd(task_core: &mut TaskCore) -> Option<ConnCmd> {
    match tokio::time::timeout(
        task_core.config.signal_config.max_idle,
        task_core.cmd_recv.recv(),
    )
    .await
    {
        Err(_) => {
            netaudit!(
                DEBUG,
                pub_key = ?task_core.pub_key,
                a = "close: connection idle",
            );
            None
        }
        Ok(None) => {
            netaudit!(
                DEBUG,
                pub_key = ?task_core.pub_key,
                a = "close: cmd_recv stream complete",
            );
            None
        }
        Ok(Some(cmd)) => Some(cmd),
    }
}

async fn webrtc_task(
    mut webrtc_recv: CloseRecv<webrtc::WebrtcEvt>,
    cmd_send: CloseSend<ConnCmd>,
) {
    while let Some(evt) = webrtc_recv.recv().await {
        if cmd_send.send(ConnCmd::WebrtcRecv(evt)).await.is_err() {
            break;
        }
    }
    let _ = cmd_send.send(ConnCmd::WebrtcClosed).await;
}

enum AttemptWebrtcResult {
    Abort,
    Fallback(TaskCore),
}

async fn con_task_attempt_webrtc(
    is_polite: bool,
    webrtc_config: Vec<u8>,
    mut task_core: TaskCore,
) -> AttemptWebrtcResult {
    use AttemptWebrtcResult::*;

    let timeout_dur = task_core.config.signal_config.max_idle;
    let timeout_cmd_send = task_core.cmd_send.clone();
    tokio::task::spawn(async move {
        tokio::time::sleep(timeout_dur).await;
        let _ = timeout_cmd_send.send(ConnCmd::WebrtcTimeoutCheck).await;
    });

    let (webrtc, webrtc_recv) = webrtc::new_backend_module(
        task_core.config.backend_module,
        is_polite,
        webrtc_config.clone(),
        // MAYBE - make this configurable
        4096,
    );

    struct AbortWebrtc(tokio::task::AbortHandle);

    impl Drop for AbortWebrtc {
        fn drop(&mut self) {
            self.0.abort();
        }
    }

    // ensure if we exit this loop that the tokio task is stopped
    let _abort_webrtc = AbortWebrtc(
        tokio::task::spawn(webrtc_task(
            webrtc_recv,
            task_core.cmd_send.clone(),
        ))
        .abort_handle(),
    );

    let mut is_ready = false;

    while let Some(cmd) = recv_cmd(&mut task_core).await {
        use tx5_signal::SignalMessage::*;
        use webrtc::WebrtcEvt::*;
        use ConnCmd::*;
        match cmd {
            SigRecv(HandshakeReq(_)) | SigRecv(HandshakeRes(_)) => {
                netaudit!(
                    DEBUG,
                    pub_key = ?task_core.pub_key,
                    a = "close: unexpected handshake msg",
                );
                return Abort;
            }
            SigRecv(tx5_signal::SignalMessage::Message(msg)) => {
                if task_core.handle_recv_msg(msg).await.is_err() {
                    return Abort;
                }
                // if we get a message from the remote, we have to assume
                // they are switching to fallback mode, and thus we cannot
                // use webrtc ourselves.
                return Fallback(task_core);
            }
            SigRecv(Offer(offer)) => {
                netaudit!(
                    TRACE,
                    pub_key = ?task_core.pub_key,
                    offer = String::from_utf8_lossy(&offer).to_string(),
                    a = "recv_offer",
                );
                if let Err(err) = webrtc.in_offer(offer).await {
                    netaudit!(
                        DEBUG,
                        pub_key = ?task_core.pub_key,
                        ?err,
                        a = "close: webrtc in_offer error",
                    );
                    return Fallback(task_core);
                }
            }
            SigRecv(Answer(answer)) => {
                netaudit!(
                    TRACE,
                    pub_key = ?task_core.pub_key,
                    offer = String::from_utf8_lossy(&answer).to_string(),
                    a = "recv_answer",
                );
                if let Err(err) = webrtc.in_answer(answer).await {
                    netaudit!(
                        DEBUG,
                        pub_key = ?task_core.pub_key,
                        ?err,
                        a = "close: webrtc in_answer error",
                    );
                    return Fallback(task_core);
                }
            }
            SigRecv(Ice(ice)) => {
                netaudit!(
                    TRACE,
                    pub_key = ?task_core.pub_key,
                    offer = String::from_utf8_lossy(&ice).to_string(),
                    a = "recv_ice",
                );
                if let Err(err) = webrtc.in_ice(ice).await {
                    netaudit!(
                        TRACE,
                        pub_key = ?task_core.pub_key,
                        ?err,
                        a = "webrtc in_ice error",
                    );
                    // ice errors are often benign... just ignore it
                }
            }
            SigRecv(WebrtcReady) | SigRecv(Keepalive) | SigRecv(Unknown) => {
                // these are all no-ops
            }
            WebrtcRecv(GeneratedOffer(offer)) => {
                netaudit!(
                    TRACE,
                    pub_key = ?task_core.pub_key,
                    offer = String::from_utf8_lossy(&offer).to_string(),
                    a = "send_offer",
                );
                if let Some(client) = task_core.client.upgrade() {
                    if let Err(err) =
                        client.send_offer(&task_core.pub_key, offer).await
                    {
                        netaudit!(
                            DEBUG,
                            pub_key = ?task_core.pub_key,
                            ?err,
                            a = "webrtc send_offer error",
                        );
                        return Abort;
                    }
                } else {
                    return Abort;
                }
            }
            WebrtcRecv(GeneratedAnswer(answer)) => {
                netaudit!(
                    TRACE,
                    pub_key = ?task_core.pub_key,
                    offer = String::from_utf8_lossy(&answer).to_string(),
                    a = "send_answer",
                );
                if let Some(client) = task_core.client.upgrade() {
                    if let Err(err) =
                        client.send_answer(&task_core.pub_key, answer).await
                    {
                        netaudit!(
                            DEBUG,
                            pub_key = ?task_core.pub_key,
                            ?err,
                            a = "webrtc send_answer error",
                        );
                        return Abort;
                    }
                } else {
                    return Abort;
                }
            }
            WebrtcRecv(GeneratedIce(ice)) => {
                netaudit!(
                    TRACE,
                    pub_key = ?task_core.pub_key,
                    offer = String::from_utf8_lossy(&ice).to_string(),
                    a = "send_ice",
                );
                if let Some(client) = task_core.client.upgrade() {
                    if let Err(err) =
                        client.send_ice(&task_core.pub_key, ice).await
                    {
                        netaudit!(
                            DEBUG,
                            pub_key = ?task_core.pub_key,
                            ?err,
                            a = "webrtc send_ice error",
                        );
                        return Abort;
                    }
                } else {
                    return Abort;
                }
            }
            WebrtcRecv(webrtc::WebrtcEvt::Message(msg)) => {
                if task_core.handle_recv_msg(msg).await.is_err() {
                    return Abort;
                }
            }
            WebrtcRecv(Ready) => {
                is_ready = true;
                task_core.is_webrtc.store(true, Ordering::SeqCst);
                task_core.ready.close();
            }
            SendMessage(msg) => {
                let len = msg.len();

                if let Err(err) = webrtc.message(msg).await {
                    netaudit!(
                        DEBUG,
                        pub_key = ?task_core.pub_key,
                        ?err,
                        a = "close: webrtc message error",
                    );
                    return Fallback(task_core);
                }

                task_core.track_send_msg(len);
            }
            WebrtcTimeoutCheck => {
                if !is_ready {
                    return Fallback(task_core);
                }
            }
            WebrtcClosed => {
                return Fallback(task_core);
            }
        }
    }

    Abort
}

async fn con_task_fallback_use_signal(mut task_core: TaskCore) {
    // closing the semaphore causes all the acquire awaits to end
    task_core.ready.close();

    while let Some(cmd) = recv_cmd(&mut task_core).await {
        match cmd {
            ConnCmd::SigRecv(tx5_signal::SignalMessage::Message(msg)) => {
                if task_core.handle_recv_msg(msg).await.is_err() {
                    break;
                }
            }
            ConnCmd::SendMessage(msg) => match task_core.client.upgrade() {
                Some(client) => {
                    let len = msg.len();
                    if let Err(err) =
                        client.send_message(&task_core.pub_key, msg).await
                    {
                        netaudit!(
                            DEBUG,
                            pub_key = ?task_core.pub_key,
                            ?err,
                            a = "close: sbd client send error",
                        );
                        break;
                    }
                    task_core.track_send_msg(len);
                }
                None => {
                    netaudit!(
                        DEBUG,
                        pub_key = ?task_core.pub_key,
                        a = "close: sbd client closed",
                    );
                    break;
                }
            },
            _ => (),
        }
    }
}
