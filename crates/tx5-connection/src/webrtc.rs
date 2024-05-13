use crate::AbortTask;
use std::io::{Error, Result};
use std::sync::{Arc, Weak};

pub enum WebrtcEvt {
    GeneratedOffer(Vec<u8>),
    GeneratedAnswer(Vec<u8>),
    GeneratedIce(Vec<u8>),
    Message(Vec<u8>),
    Ready,
}

enum Cmd {
    InOffer(Vec<u8>),
    InAnswer(Vec<u8>),
    InIce(Vec<u8>),
    GeneratedIce(Vec<u8>),
    DataChan(
        tx5_go_pion::DataChannel,
        tokio::sync::mpsc::UnboundedReceiver<tx5_go_pion::DataChannelEvent>,
    ),
    SendMessage(Vec<u8>, tokio::sync::oneshot::Sender<()>),
    RecvMessage(Vec<u8>),
    DataChanOpen,
    BufferedAmountLow,
}

async fn weak_send<T>(
    s: &Weak<tokio::sync::mpsc::Sender<T>>,
    t: T,
) -> Result<()> {
    if let Some(s) = s.upgrade() {
        s.send(t).await.map_err(|_| Error::other("closed"))
    } else {
        Err(Error::other("closed"))
    }
}

pub struct Webrtc {
    cmd_send: Arc<tokio::sync::mpsc::Sender<Cmd>>,
    _task: AbortTask<Result<()>>,
    _evt_send: Arc<tokio::sync::mpsc::Sender<WebrtcEvt>>,
}

impl Webrtc {
    pub fn new(
        is_polite: bool,
        config: Vec<u8>,
        send_buffer: usize,
    ) -> (Self, tokio::sync::mpsc::Receiver<WebrtcEvt>) {
        let (cmd_send, cmd_recv) = tokio::sync::mpsc::channel(32);
        let cmd_send = Arc::new(cmd_send);
        let (evt_send, evt_recv) = tokio::sync::mpsc::channel(32);
        let evt_send = Arc::new(evt_send);

        let task = tokio::task::spawn(task(
            is_polite,
            config,
            send_buffer,
            Arc::downgrade(&evt_send),
            Arc::downgrade(&cmd_send),
            cmd_recv,
        ));

        (
            Self {
                cmd_send,
                _task: AbortTask(task),
                _evt_send: evt_send,
            },
            evt_recv,
        )
    }

    pub async fn in_offer(&self, offer: Vec<u8>) -> Result<()> {
        self.cmd_send
            .send(Cmd::InOffer(offer))
            .await
            .map_err(|_| Error::other("closed"))
    }

    pub async fn in_answer(&self, answer: Vec<u8>) -> Result<()> {
        self.cmd_send
            .send(Cmd::InAnswer(answer))
            .await
            .map_err(|_| Error::other("closed"))
    }

    pub async fn in_ice(&self, ice: Vec<u8>) -> Result<()> {
        self.cmd_send
            .send(Cmd::InIce(ice))
            .await
            .map_err(|_| Error::other("closed"))
    }

    pub async fn message(&self, message: Vec<u8>) -> Result<()> {
        let (s, r) = tokio::sync::oneshot::channel();
        self.cmd_send
            .send(Cmd::SendMessage(message, s))
            .await
            .map_err(|_| Error::other("closed"))?;
        let _ = r.await;
        Ok(())
    }
}

async fn task(
    is_polite: bool,
    config: Vec<u8>,
    send_buffer: usize,
    evt_send: Weak<tokio::sync::mpsc::Sender<WebrtcEvt>>,
    cmd_send: Weak<tokio::sync::mpsc::Sender<Cmd>>,
    mut cmd_recv: tokio::sync::mpsc::Receiver<Cmd>,
) -> Result<()> {
    let (peer, mut peer_evt) = tx5_go_pion::PeerConnection::new(config).await?;

    let cmd_send2 = cmd_send.clone();
    let _peer_task: AbortTask<Result<()>> =
        AbortTask(tokio::task::spawn(async move {
            use tx5_go_pion::PeerConnectionEvent as Evt;
            while let Some(evt) = peer_evt.recv().await {
                match evt {
                    Evt::Error(_) => break,
                    Evt::State(_) => (),
                    Evt::ICECandidate(mut ice) => {
                        weak_send(&cmd_send2, Cmd::GeneratedIce(ice.to_vec()?))
                            .await?;
                    }
                    Evt::DataChannel(d, dr) => {
                        weak_send(&cmd_send2, Cmd::DataChan(d, dr)).await?;
                    }
                }
            }
            Ok(())
        }));

    let mut offer = None;
    let mut data = None;
    let mut _data_recv = None;
    let mut did_handshake = false;
    let mut pend_buffer = Vec::new();

    if !is_polite {
        let (d, dr) = peer
            .create_data_channel(b"{\"label\":\"data\"}".to_vec())
            .await?;
        d.set_buffered_amount_low_threshold(send_buffer)?;
        data = Some(d);
        _data_recv = spawn_data_chan(cmd_send.clone(), dr);
        let mut o = peer.create_offer(b"{}".to_vec()).await?;
        weak_send(&evt_send, WebrtcEvt::GeneratedOffer(o.to_vec()?)).await?;
        offer = Some(o);
    }

    while let Some(cmd) = cmd_recv.recv().await {
        match cmd {
            Cmd::InOffer(o) => {
                if is_polite && !did_handshake {
                    peer.set_remote_description(o).await?;
                    let mut a = peer.create_answer(b"{}".to_vec()).await?;
                    weak_send(
                        &evt_send,
                        WebrtcEvt::GeneratedAnswer(a.to_vec()?),
                    )
                    .await?;
                    peer.set_local_description(a).await?;
                    did_handshake = true;
                }
            }
            Cmd::InAnswer(a) => {
                if !is_polite && !did_handshake {
                    if let Some(o) = offer.take() {
                        peer.set_local_description(o).await?;
                        peer.set_remote_description(a).await?;
                        did_handshake = true;
                    }
                }
            }
            Cmd::InIce(i) => {
                let _ = peer.add_ice_candidate(i).await;
            }
            Cmd::GeneratedIce(ice) => {
                weak_send(&evt_send, WebrtcEvt::GeneratedIce(ice)).await?;
            }
            Cmd::DataChan(d, dr) => {
                if data.is_none() {
                    d.set_buffered_amount_low_threshold(send_buffer)?;
                    data = Some(d);
                    _data_recv = spawn_data_chan(cmd_send.clone(), dr);
                }
            }
            Cmd::SendMessage(msg, resp) => {
                if let Some(d) = &data {
                    let amt = match d.send(msg).await {
                        Ok(amt) => amt,
                        Err(_) => break,
                    };
                    if amt <= send_buffer {
                        drop(resp);
                    } else {
                        pend_buffer.push(resp);
                    }
                } else {
                    break;
                }
            }
            Cmd::RecvMessage(msg) => {
                weak_send(&evt_send, WebrtcEvt::Message(msg)).await?;
            }
            Cmd::DataChanOpen => {
                weak_send(&evt_send, WebrtcEvt::Ready).await?;
            }
            Cmd::BufferedAmountLow => {
                pend_buffer.clear();
            }
        }
    }

    Ok(())
}

/// Receiving on the unbounded data channel receiver has a real
/// chance to fill our memory with message data.
/// We give a small chance for the app to catch up, otherwise
/// error so the connection will close.
async fn weak_send_crit<T>(
    s: &Weak<tokio::sync::mpsc::Sender<T>>,
    t: T,
) -> Result<()> {
    if let Some(s) = s.upgrade() {
        s.send_timeout(t, std::time::Duration::from_millis(15))
            .await
            .map_err(|_| Error::other("closed"))
    } else {
        Err(Error::other("closed"))
    }
}

fn spawn_data_chan(
    cmd_send: Weak<tokio::sync::mpsc::Sender<Cmd>>,
    mut data_recv: tokio::sync::mpsc::UnboundedReceiver<
        tx5_go_pion::DataChannelEvent,
    >,
) -> Option<AbortTask<Result<()>>> {
    use tx5_go_pion::DataChannelEvent as Evt;
    Some(AbortTask(tokio::task::spawn(async move {
        while let Some(evt) = data_recv.recv().await {
            match evt {
                Evt::Error(_) => break,
                Evt::Open => {
                    weak_send_crit(&cmd_send, Cmd::DataChanOpen).await?;
                }
                Evt::Close => break,
                Evt::Message(mut msg) => {
                    weak_send_crit(&cmd_send, Cmd::RecvMessage(msg.to_vec()?))
                        .await?;
                }
                Evt::BufferedAmountLow => {
                    weak_send_crit(&cmd_send, Cmd::BufferedAmountLow).await?;
                }
            }
        }
        Ok(())
    })))
}
