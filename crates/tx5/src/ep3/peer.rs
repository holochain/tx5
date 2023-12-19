use super::*;

pub(crate) enum PeerCmd {
    Error(Error),
    SigRecvIce(serde_json::Value),
}

impl From<Error> for PeerCmd {
    fn from(err: Error) -> Self {
        Self::Error(err)
    }
}

pub(crate) enum PeerDir {
    ActiveOrOutgoing,
    Incoming { offer: serde_json::Value },
}

impl PeerDir {
    pub fn is_incoming(&self) -> bool {
        matches!(self, PeerDir::Incoming { .. })
    }
}

pub(crate) enum NewPeerDir {
    Outgoing {
        answer_recv: tokio::sync::oneshot::Receiver<serde_json::Value>,
    },
    Incoming {
        offer: serde_json::Value,
    },
}

pub(crate) struct Peer {
    sig: Arc<SigShared>,
    peer_id: Id,
    peer_uniq: u64,
    _permit: tokio::sync::OwnedSemaphorePermit,
    cmd_task: tokio::task::JoinHandle<()>,
    recv_task: tokio::task::JoinHandle<()>,
    data_task: tokio::task::JoinHandle<()>,
    #[allow(dead_code)]
    peer: Arc<tx5_go_pion::PeerConnection>,
    data_chan: Arc<tx5_go_pion::DataChannel>,
    send_limit: Arc<tokio::sync::Semaphore>,
}

impl Drop for Peer {
    fn drop(&mut self) {
        tracing::info!(%self.sig.ep_uniq, %self.sig.sig_uniq, %self.peer_uniq, ?self.peer_id, "Peer Connection Close");

        self.cmd_task.abort();
        self.recv_task.abort();
        self.data_task.abort();

        close_peer(&self.sig.weak_peer_map, self.peer_id, self.peer_uniq);
    }
}

impl Peer {
    pub async fn new(
        sig: Arc<SigShared>,
        peer_url: PeerUrl,
        peer_id: Id,
        peer_uniq: u64,
        ice_servers: Arc<serde_json::Value>,
        new_peer_dir: NewPeerDir,
        mut peer_cmd_recv: EventRecv<PeerCmd>,
    ) -> CRes<Arc<Self>> {
        tracing::info!(%sig.ep_uniq, %sig.sig_uniq, %peer_uniq, ?peer_id, "Peer Connection Connecting");

        let _permit =
            sig.peer_limit.clone().acquire_owned().await.map_err(|_| {
                Error::str(
                    "Endpoint closed while acquiring peer connection permit",
                )
            })?;

        let sig_hnd = match sig.weak_sig.upgrade() {
            None => {
                return Err(Error::str(
                    "Sig shutdown while opening peer connection",
                )
                .into())
            }
            Some(sig_hnd) => sig_hnd,
        };

        let peer_config = BackBuf::from_json(ice_servers)?;

        let (peer, mut peer_recv) = tx5_go_pion::PeerConnection::new(
            peer_config.imp.buf,
            Arc::new(tokio::sync::Semaphore::new(
                sig.config.connection_bytes_max as usize,
            )),
        )
        .await?;

        let peer = Arc::new(peer);

        let (chan_send, chan_recv) = tokio::sync::oneshot::channel();
        let mut chan_send = Some(chan_send);

        match new_peer_dir {
            NewPeerDir::Outgoing { answer_recv } => {
                let chan = peer
                    .create_data_channel(tx5_go_pion::DataChannelConfig {
                        label: Some("data".into()),
                    })
                    .await?;

                if let Some(chan_send) = chan_send.take() {
                    let _ = chan_send.send(chan);
                }

                let mut offer = peer
                    .create_offer(tx5_go_pion::OfferConfig::default())
                    .await?;

                let offer_json = offer.as_json()?;

                tracing::debug!(?offer_json, "create_offer");

                sig_hnd.offer(peer_id, offer_json).await?;

                peer.set_local_description(offer).await?;

                let answer = answer_recv.await.map_err(|_| {
                    Error::str("Failed to receive answer on peer connect")
                })?;
                let answer = BackBuf::from_json(answer)?;

                peer.set_remote_description(answer.imp.buf).await?;
            }
            NewPeerDir::Incoming { offer } => {
                let offer = BackBuf::from_json(offer)?;

                peer.set_remote_description(offer.imp.buf).await?;

                let mut answer = peer
                    .create_answer(tx5_go_pion::AnswerConfig::default())
                    .await?;

                let answer_json = answer.as_json()?;

                tracing::debug!(?answer_json, "create_answer");

                sig_hnd.answer(peer_id, answer_json).await?;

                peer.set_local_description(answer).await?;
            }
        }

        let cmd_task = {
            let weak_peer = Arc::downgrade(&peer);
            let sig = sig.clone();
            tokio::task::spawn(async move {
                while let Some(cmd) = peer_cmd_recv.recv().await {
                    match cmd {
                        PeerCmd::Error(err) => {
                            tracing::warn!(?err);
                            break;
                        }
                        PeerCmd::SigRecvIce(ice) => {
                            if let Some(peer) = weak_peer.upgrade() {
                                if let Ok(ice) = BackBuf::from_json(ice) {
                                    if let Err(err) = peer
                                        .add_ice_candidate(ice.imp.buf)
                                        .await
                                    {
                                        tracing::trace!(?err);
                                    }
                                }
                            } else {
                                break;
                            }
                        }
                    }
                }

                close_peer(&sig.weak_peer_map, peer_id, peer_uniq);
            })
        };

        let recv_task = {
            let sig = sig.clone();
            tokio::task::spawn(async move {
                while let Some(evt) = peer_recv.recv().await {
                    use tx5_go_pion::PeerConnectionEvent as Evt;
                    match evt {
                        Evt::Error(err) => {
                            tracing::warn!(?err);
                            break;
                        }
                        Evt::State(_state) => (),
                        Evt::ICECandidate(mut ice) => {
                            let ice = match ice.as_json() {
                                Err(err) => {
                                    tracing::warn!(?err, "invalid ice");
                                    break;
                                }
                                Ok(ice) => ice,
                            };
                            if let Some(sig_hnd) = sig.weak_sig.upgrade() {
                                if sig_hnd.ice(peer_id, ice).await.is_err() {
                                    break;
                                }
                            } else {
                                break;
                            }
                        }
                        Evt::DataChannel(c, r) => {
                            if let Some(chan_send) = chan_send.take() {
                                let _ = chan_send.send((c, r));
                            } else {
                                tracing::warn!("Invalid incoming data channel");
                                break;
                            }
                        }
                    }
                }

                close_peer(&sig.weak_peer_map, peer_id, peer_uniq);
            })
        };

        let (data_chan, mut data_recv) = chan_recv.await.map_err(|_| {
            Error::str("Failed to establish peer connection data channel")
        })?;

        let data_chan = Arc::new(data_chan);

        let data_task = {
            let sig = sig.clone();
            tokio::task::spawn(async move {
                while let Some(evt) = data_recv.recv().await {
                    use tx5_go_pion::DataChannelEvent::*;
                    match evt {
                        Error(err) => {
                            tracing::warn!(?err);
                            break;
                        }
                        Open => (),
                        Close => break,
                        Message(message, permit) => {
                            let message = BackBuf::from_raw(message);
                            if sig
                                .evt_send
                                .send(Ep3Event::Message {
                                    peer_url: peer_url.clone(),
                                    message,
                                    permit,
                                })
                                .await
                                .is_err()
                            {
                                break;
                            }
                        }
                        BufferedAmountLow => (),
                    }
                }

                close_peer(&sig.weak_peer_map, peer_id, peer_uniq);
            })
        };

        let mut ready_state = data_chan.ready_state()?;
        let mut backoff = std::time::Duration::from_millis(10);

        loop {
            if ready_state >= 2 {
                break;
            }

            tokio::time::sleep(backoff).await;
            backoff *= 2;
            ready_state = data_chan.ready_state()?;
        }

        if ready_state > 2 {
            return Err(Error::str(
                "Data channel closed while connecting peer",
            )
            .into());
        }

        tracing::info!(%sig.ep_uniq, %sig.sig_uniq, %peer_uniq, ?peer_id, "Peer Connection Open");

        Ok(Arc::new(Self {
            sig,
            peer_id,
            peer_uniq,
            _permit,
            cmd_task,
            recv_task,
            data_task,
            peer,
            data_chan,
            send_limit: Arc::new(tokio::sync::Semaphore::new(1)),
        }))
    }

    pub async fn send(&self, data: Vec<BackBuf>) -> Result<()> {
        // size 1 semaphore makes sure blocks of messages are contiguous
        let _permit = self
            .send_limit
            .acquire()
            .await
            .map_err(|_| Error::str("Failed to acquire send permit"))?;

        for mut buf in data {
            if buf.len()? > 16 * 1024 {
                return Err(Error::str("Buffer cannot be larger than 16 KiB"));
            }

            let mut backoff = std::time::Duration::from_millis(1);

            loop {
                if self.data_chan.buffered_amount()?
                    <= self.sig.config.connection_bytes_max as usize
                {
                    break;
                }

                tokio::time::sleep(backoff).await;
                backoff *= 2;
            }

            self.data_chan.send(buf.imp.buf).await?;
        }

        Ok(())
    }
}

