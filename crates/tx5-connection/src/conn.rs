use super::*;
use std::sync::atomic::Ordering;

/// Message count allowed to accumulate on the boundary switching to webrtc.
/// This prevents potentially faster to transfer webrtc messages from being
/// delivered out of order before trailing slower sbd messages are received.
const MAX_WEBRTC_BUF: usize = 512;

pub(crate) enum ConnCmd {
    SigRecv(tx5_signal::SignalMessage),
    SendMessage(Vec<u8>),
    WebrtcMessage(Vec<u8>),
    WebrtcReady,
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
    webrtc_ready: Arc<tokio::sync::Semaphore>,
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

        let send_msg_count = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let send_byte_count = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let recv_msg_count = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let recv_byte_count = Arc::new(std::sync::atomic::AtomicU64::new(0));

        // zero len semaphore.. we actually just wait for the close
        let ready = Arc::new(tokio::sync::Semaphore::new(0));
        let webrtc_ready = Arc::new(tokio::sync::Semaphore::new(0));

        let (mut msg_send, msg_recv) = CloseSend::sized_channel(1024);
        let (cmd_send, mut cmd_recv) = CloseSend::sized_channel(1024);

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

        let send_msg_count2 = send_msg_count.clone();
        let send_byte_count2 = send_byte_count.clone();
        let recv_msg_count2 = recv_msg_count.clone();
        let recv_byte_count2 = recv_byte_count.clone();
        let webrtc_ready2 = webrtc_ready.clone();
        let ready2 = ready.clone();
        let client2 = client.clone();
        let pub_key2 = pub_key.clone();
        let mut cmd_send3 = cmd_send.clone();
        let conn_task = tokio::task::spawn(async move {
            let client = match client2.upgrade() {
                Some(client) => client,
                None => return,
            };

            let mut webrtc_message_buffer = Vec::new();

            let handshake_fut = async {
                let nonce = client.send_handshake_req(&pub_key2).await?;

                let mut got_peer_res = false;
                let mut sent_our_res = false;

                while let Some(cmd) = cmd_recv.recv().await {
                    match cmd {
                        ConnCmd::SigRecv(sig) => {
                            use tx5_signal::SignalMessage::*;
                            match sig {
                                HandshakeReq(oth_nonce) => {
                                    client
                                        .send_handshake_res(
                                            &pub_key2, oth_nonce,
                                        )
                                        .await?;
                                    sent_our_res = true;
                                }
                                HandshakeRes(res_nonce) => {
                                    if res_nonce != nonce {
                                        return Err(Error::other(
                                            "nonce mismatch",
                                        ));
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
                        ConnCmd::WebrtcMessage(msg) => {
                            webrtc_message_buffer.push(msg);
                        }
                        ConnCmd::WebrtcReady => {
                            webrtc_ready2.close();
                        }
                        ConnCmd::WebrtcClosed => {
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
                config.signal_config.max_idle,
                handshake_fut,
            )
            .await
            {
                Err(_) | Ok(Err(_)) => {
                    client.close_peer(&pub_key2).await;
                    return;
                }
                Ok(Ok(_)) => (),
            }

            drop(client);

            // closing the semaphore causes all the acquire awaits to end
            ready2.close();

            let (webrtc, mut webrtc_recv) = webrtc::new_backend_module(
                config.backend_module,
                is_polite,
                webrtc_config,
                // MAYBE - make this configurable
                4096,
            );

            let client3 = client2.clone();
            let pub_key3 = pub_key2.clone();
            let _webrtc_task = AbortTask(tokio::task::spawn(async move {
                use webrtc::WebrtcEvt::*;
                cmd_send3.set_close_on_drop(true);
                while let Some(evt) = webrtc_recv.recv().await {
                    match evt {
                        GeneratedOffer(offer) => {
                            netaudit!(
                                TRACE,
                                pub_key = ?pub_key3,
                                offer = String::from_utf8_lossy(&offer).to_string(),
                                a = "send_offer",
                            );
                            if let Some(client) = client3.upgrade() {
                                if let Err(err) =
                                    client.send_offer(&pub_key3, offer).await
                                {
                                    netaudit!(
                                        DEBUG,
                                        pub_key = ?pub_key3,
                                        ?err,
                                        a = "webrtc send_offer error",
                                    );
                                    break;
                                }
                            } else {
                                break;
                            }
                        }
                        GeneratedAnswer(answer) => {
                            netaudit!(
                                TRACE,
                                pub_key = ?pub_key3,
                                offer = String::from_utf8_lossy(&answer).to_string(),
                                a = "send_answer",
                            );
                            if let Some(client) = client3.upgrade() {
                                if let Err(err) =
                                    client.send_answer(&pub_key3, answer).await
                                {
                                    netaudit!(
                                        DEBUG,
                                        pub_key = ?pub_key3,
                                        ?err,
                                        a = "webrtc send_answer error",
                                    );
                                    break;
                                }
                            } else {
                                break;
                            }
                        }
                        GeneratedIce(ice) => {
                            netaudit!(
                                TRACE,
                                pub_key = ?pub_key3,
                                offer = String::from_utf8_lossy(&ice).to_string(),
                                a = "send_ice",
                            );
                            if let Some(client) = client3.upgrade() {
                                if let Err(err) =
                                    client.send_ice(&pub_key3, ice).await
                                {
                                    netaudit!(
                                        DEBUG,
                                        pub_key = ?pub_key3,
                                        ?err,
                                        a = "webrtc send_ice error",
                                    );
                                    break;
                                }
                            } else {
                                break;
                            }
                        }
                        Message(msg) => {
                            if cmd_send3
                                .send(ConnCmd::WebrtcMessage(msg))
                                .await
                                .is_err()
                            {
                                netaudit!(
                                    DEBUG,
                                    pub_key = ?pub_key3,
                                    a = "webrtc cmd closed",
                                );
                                break;
                            }
                        }
                        Ready => {
                            if cmd_send3
                                .send(ConnCmd::WebrtcReady)
                                .await
                                .is_err()
                            {
                                netaudit!(
                                    DEBUG,
                                    pub_key = ?pub_key3,
                                    a = "webrtc cmd closed",
                                );
                                break;
                            }
                        }
                    }
                }

                let _ = cmd_send3.send(ConnCmd::WebrtcClosed).await;
            }));

            msg_send.set_close_on_drop(true);

            let mut recv_over_webrtc = false;
            let mut send_over_webrtc = false;

            loop {
                let cmd = tokio::time::timeout(
                    config.signal_config.max_idle,
                    cmd_recv.recv(),
                )
                .await;

                let cmd = match cmd {
                    Err(_) => {
                        netaudit!(
                            DEBUG,
                            pub_key = ?pub_key2,
                            a = "close: connection idle",
                        );
                        break;
                    }
                    Ok(cmd) => cmd,
                };

                let cmd = match cmd {
                    None => {
                        netaudit!(
                            DEBUG,
                            pub_key = ?pub_key2,
                            a = "close: cmd_recv stream complete",
                        );
                        break;
                    }
                    Some(cmd) => cmd,
                };

                match cmd {
                    ConnCmd::SigRecv(sig) => {
                        use tx5_signal::SignalMessage::*;
                        match sig {
                            // invalid
                            HandshakeReq(_) | HandshakeRes(_) => {
                                netaudit!(
                                    DEBUG,
                                    pub_key = ?pub_key2,
                                    a = "close: unexpected handshake msg",
                                );
                                break;
                            }
                            Message(msg) => {
                                if recv_over_webrtc {
                                    netaudit!(
                                        DEBUG,
                                        pub_key = ?pub_key2,
                                        a = "close: unexpected sbd msg after recv_over_webrtc",
                                    );
                                    break;
                                } else {
                                    recv_msg_count2
                                        .fetch_add(1, Ordering::Relaxed);
                                    recv_byte_count2.fetch_add(
                                        msg.len() as u64,
                                        Ordering::Relaxed,
                                    );
                                    if msg_send.send(msg).await.is_err() {
                                        netaudit!(
                                            DEBUG,
                                            pub_key = ?pub_key2,
                                            a = "close: msg_send closed",
                                        );
                                        break;
                                    }
                                }
                            }
                            Offer(offer) => {
                                netaudit!(
                                    TRACE,
                                    pub_key = ?pub_key2,
                                    offer = String::from_utf8_lossy(&offer).to_string(),
                                    a = "recv_offer",
                                );
                                if let Err(err) = webrtc.in_offer(offer).await {
                                    netaudit!(
                                        DEBUG,
                                        pub_key = ?pub_key2,
                                        ?err,
                                        a = "close: webrtc in_offer error",
                                    );
                                    break;
                                }
                            }
                            Answer(answer) => {
                                netaudit!(
                                    TRACE,
                                    pub_key = ?pub_key2,
                                    offer = String::from_utf8_lossy(&answer).to_string(),
                                    a = "recv_answer",
                                );
                                if let Err(err) = webrtc.in_answer(answer).await
                                {
                                    netaudit!(
                                        DEBUG,
                                        pub_key = ?pub_key2,
                                        ?err,
                                        a = "close: webrtc in_answer error",
                                    );
                                    break;
                                }
                            }
                            Ice(ice) => {
                                netaudit!(
                                    TRACE,
                                    pub_key = ?pub_key2,
                                    offer = String::from_utf8_lossy(&ice).to_string(),
                                    a = "recv_ice",
                                );
                                if let Err(err) = webrtc.in_ice(ice).await {
                                    netaudit!(
                                        DEBUG,
                                        pub_key = ?pub_key2,
                                        ?err,
                                        a = "close: webrtc in_ice error",
                                    );
                                    break;
                                }
                            }
                            WebrtcReady => {
                                recv_over_webrtc = true;
                                netaudit!(
                                    DEBUG,
                                    pub_key = ?pub_key2,
                                    a = "recv_over_webrtc",
                                );
                                for msg in webrtc_message_buffer.drain(..) {
                                    // don't bump send metrics here,
                                    // we bumped them on receive
                                    if msg_send.send(msg).await.is_err() {
                                        netaudit!(
                                            DEBUG,
                                            pub_key = ?pub_key2,
                                            a = "close: msg_send closed",
                                        );
                                        break;
                                    }
                                }
                            }
                            _ => (),
                        }
                    }
                    ConnCmd::SendMessage(msg) => {
                        send_msg_count2.fetch_add(1, Ordering::Relaxed);
                        send_byte_count2
                            .fetch_add(msg.len() as u64, Ordering::Relaxed);
                        if send_over_webrtc {
                            if let Err(err) = webrtc.message(msg).await {
                                netaudit!(
                                    DEBUG,
                                    pub_key = ?pub_key2,
                                    ?err,
                                    a = "close: webrtc message error",
                                );
                                break;
                            }
                        } else if let Some(client) = client2.upgrade() {
                            if let Err(err) =
                                client.send_message(&pub_key2, msg).await
                            {
                                netaudit!(
                                    DEBUG,
                                    pub_key = ?pub_key2,
                                    ?err,
                                    a = "close: sbd client send error",
                                );
                                break;
                            }
                        } else {
                            netaudit!(
                                DEBUG,
                                pub_key = ?pub_key2,
                                a = "close: sbd client closed",
                            );
                            break;
                        }
                    }
                    ConnCmd::WebrtcMessage(msg) => {
                        recv_msg_count2.fetch_add(1, Ordering::Relaxed);
                        recv_byte_count2
                            .fetch_add(msg.len() as u64, Ordering::Relaxed);
                        if recv_over_webrtc {
                            if msg_send.send(msg).await.is_err() {
                                netaudit!(
                                    DEBUG,
                                    pub_key = ?pub_key2,
                                    a = "close: msg_send closed",
                                );
                                break;
                            }
                        } else {
                            webrtc_message_buffer.push(msg);
                            if webrtc_message_buffer.len() > MAX_WEBRTC_BUF {
                                // prevent memory fillup
                                netaudit!(
                                    DEBUG,
                                    pub_key = ?pub_key2,
                                    a = "close: webrtc buffer overflow",
                                );
                                break;
                            }
                        }
                    }
                    ConnCmd::WebrtcReady => {
                        if let Some(client) = client2.upgrade() {
                            if let Err(err) =
                                client.send_webrtc_ready(&pub_key2).await
                            {
                                netaudit!(
                                    DEBUG,
                                    pub_key = ?pub_key2,
                                    ?err,
                                    a = "close: sbd client ready error",
                                );
                                break;
                            }
                        } else {
                            netaudit!(
                                DEBUG,
                                pub_key = ?pub_key2,
                                a = "close: sbd client closed",
                            );
                            break;
                        }
                        send_over_webrtc = true;
                        netaudit!(
                            DEBUG,
                            pub_key = ?pub_key2,
                            a = "send_over_webrtc",
                        );
                        webrtc_ready2.close();
                    }
                    ConnCmd::WebrtcClosed => {
                        netaudit!(
                            DEBUG,
                            pub_key = ?pub_key2,
                            a = "close: webrtc closed",
                        );
                        break;
                    }
                }
            }

            // explicitly close the peer
            if let Some(client) = client2.upgrade() {
                client.close_peer(&pub_key2).await;
            };

            // the receiver side is closed because msg_send is dropped.
        });

        let mut cmd_send2 = cmd_send.clone();
        cmd_send2.set_close_on_drop(true);
        let this = Self {
            ready,
            pub_key,
            cmd_send: cmd_send2,
            conn_task,
            keepalive_task,
            webrtc_ready,
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

    /// Wait until this connection is connected via webrtc.
    /// Note, this will never resolve if we never successfully
    /// connect over webrtc.
    pub async fn webrtc_ready(&self) {
        let _ = self.webrtc_ready.acquire().await;
    }

    /// Returns `true` if we sucessfully connected over webrtc.
    pub fn is_using_webrtc(&self) -> bool {
        self.webrtc_ready.is_closed()
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
