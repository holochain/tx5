use super::*;

type PeerCmdSend = EventSend<PeerCmd>;
type AnswerSend =
    Arc<Mutex<Option<tokio::sync::oneshot::Sender<serde_json::Value>>>>;
type SharedPeer = Shared<BoxFuture<'static, CRes<Arc<Peer>>>>;
type PeerMap = HashMap<Id, (u64, PeerCmdSend, AnswerSend, SharedPeer)>;

pub(crate) struct SigShared {
    pub ep: Arc<EpShared>,
    pub weak_sig: Weak<Sig>,
    pub sig_uniq: u64,
    pub sig_url: SigUrl,
    pub weak_peer_map: Weak<Mutex<PeerMap>>,
}

impl std::ops::Deref for SigShared {
    type Target = Arc<EpShared>;

    fn deref(&self) -> &Self::Target {
        &self.ep
    }
}

pub(crate) struct Sig {
    sig: Arc<SigShared>,
    _permit: tokio::sync::OwnedSemaphorePermit,
    recv_task: tokio::task::JoinHandle<()>,
    ice_servers: Arc<serde_json::Value>,
    peer_map: Arc<Mutex<PeerMap>>,
    sig_cli: tx5_signal::Cli,
}

impl Drop for Sig {
    fn drop(&mut self) {
        tracing::info!(%self.sig.ep_uniq, %self.sig.sig_uniq, %self.sig.sig_url, "Signal Connection Close");

        self.recv_task.abort();

        close_sig(&self.sig.weak_sig_map, &self.sig.sig_url, self.sig.sig_uniq);
    }
}

impl std::ops::Deref for Sig {
    type Target = tx5_signal::Cli;

    fn deref(&self) -> &Self::Target {
        &self.sig_cli
    }
}

impl Sig {
    pub async fn new(
        ep: Arc<EpShared>,
        sig_uniq: u64,
        sig_url: SigUrl,
    ) -> CRes<Arc<Self>> {
        tracing::info!(%ep.ep_uniq, %sig_uniq, %sig_url, "Signal Connection Connecting");

        let _permit =
            ep.sig_limit.clone().acquire_owned().await.map_err(|_| {
                Error::str(
                    "Endpoint closed while acquiring signal connection permit",
                )
            })?;

        let (sig_cli, mut sig_recv) = tx5_signal::Cli::builder()
            .with_lair_tag(ep.lair_tag.clone())
            .with_lair_client(ep.lair_client.clone())
            .with_url(sig_url.to_string().parse().unwrap())
            .build()
            .await?;

        let peer_url = Tx5Url::new(sig_cli.local_addr())?;
        if peer_url.id().unwrap() != ep.this_id {
            return Err(Error::str("Invalid signal server peer Id").into());
        }

        let ice_servers = sig_cli.ice_servers();

        let peer_map: Arc<Mutex<PeerMap>> =
            Arc::new(Mutex::new(HashMap::new()));
        let weak_peer_map = Arc::downgrade(&peer_map);

        Ok(Arc::new_cyclic(move |weak_sig: &Weak<Sig>| {
            let recv_task = {
                let ep = ep.clone();
                let weak_sig = weak_sig.clone();
                let sig_url = sig_url.clone();
                tokio::task::spawn(async move {
                    while let Some(msg) = sig_recv.recv().await {
                        use tx5_signal::SignalMsg::*;
                        match msg {
                            Demo { .. } => (),
                            Offer { rem_pub, offer } => {
                                tracing::trace!(%ep.ep_uniq, %sig_uniq, ?rem_pub, ?offer, "Sig Recv Offer");
                                if let Some(sig) = weak_sig.upgrade() {
                                    let peer_url = sig_url.to_client(rem_pub);
                                    // fire and forget this
                                    tokio::task::spawn(async move {
                                        let _ = sig
                                            .assert_peer(
                                                peer_url,
                                                rem_pub,
                                                PeerDir::Incoming { offer },
                                            )
                                            .await;
                                    });
                                } else {
                                    break;
                                }
                            }
                            Answer { rem_pub, answer } => {
                                tracing::trace!(%ep.ep_uniq, %sig_uniq, ?rem_pub, ?answer, "Sig Recv Answer");
                                if let Some(peer_map) = weak_peer_map.upgrade()
                                {
                                    let r = peer_map
                                        .lock()
                                        .unwrap()
                                        .get(&rem_pub)
                                        .cloned();
                                    if let Some((_, _, answer_send, _)) = r {
                                        let r =
                                            answer_send.lock().unwrap().take();
                                        if let Some(answer_send) = r {
                                            let _ = answer_send.send(answer);
                                        }
                                    }
                                } else {
                                    break;
                                }
                            }
                            Ice { rem_pub, ice } => {
                                tracing::trace!(%ep.ep_uniq, %sig_uniq, ?rem_pub, ?ice, "Sig Recv ICE");
                                if let Some(peer_map) = weak_peer_map.upgrade()
                                {
                                    let r = peer_map
                                        .lock()
                                        .unwrap()
                                        .get(&rem_pub)
                                        .cloned();
                                    if let Some((_, peer_cmd_send, _, _)) = r {
                                        if let Some(permit) =
                                            peer_cmd_send.try_permit()
                                        {
                                            if peer_cmd_send
                                                .send_permit(
                                                    PeerCmd::SigRecvIce(ice),
                                                    permit,
                                                )
                                                .is_err()
                                            {
                                                break;
                                            }
                                        } else {
                                            break;
                                        }
                                    }
                                } else {
                                    break;
                                }
                            }
                        }
                    }

                    close_sig(&ep.weak_sig_map, &sig_url, sig_uniq);
                })
            };

            let weak_peer_map = Arc::downgrade(&peer_map);

            tracing::info!(%ep.ep_uniq, %sig_uniq, %sig_url, "Signal Connection Open");

            Self {
                sig: Arc::new(SigShared {
                    ep,
                    weak_sig: weak_sig.clone(),
                    sig_uniq,
                    sig_url,
                    weak_peer_map,
                }),
                _permit,
                recv_task,
                ice_servers,
                peer_map,
                sig_cli,
            }
        }))
    }

    pub async fn assert_peer(
        &self,
        peer_url: PeerUrl,
        peer_id: Id,
        peer_dir: PeerDir,
    ) -> CRes<Arc<Peer>> {
        if peer_id == self.sig.this_id {
            return Err(Error::str("Cannot establish connection with remote peer id matching this id").into());
        }

        if self.sig.ban_map.lock().unwrap().is_banned(peer_id) {
            return Err(Error::str("Peer is currently banned").into());
        }

        let mut tmp = None;

        let (_peer_uniq, _peer_cmd_send, _answer_send, fut) = {
            let mut lock = self.peer_map.lock().unwrap();

            if peer_dir.is_incoming() && lock.contains_key(&peer_id) {
                // we need to check negotiation
                if peer_id > self.sig.this_id {
                    // we are the polite node, drop our existing connection
                    tmp = lock.remove(&peer_id);
                }
                // otherwise continue on to return the currently
                // registered connection because we're the impolite node.
            }

            lock.entry(peer_id)
                .or_insert_with(|| {
                    let mut answer_send = None;
                    let new_peer_dir = match peer_dir {
                        PeerDir::ActiveOrOutgoing => {
                            let (s, r) = tokio::sync::oneshot::channel();
                            answer_send = Some(s);
                            NewPeerDir::Outgoing { answer_recv: r }
                        }
                        PeerDir::Incoming { offer } => {
                            NewPeerDir::Incoming { offer }
                        }
                    };
                    let sig = self.sig.clone();
                    let peer_uniq = next_uniq();
                    let ice_servers = self.ice_servers.clone();
                    let (peer_cmd_send, peer_cmd_recv) = EventSend::new(1024);
                    (
                        peer_uniq,
                        peer_cmd_send,
                        Arc::new(Mutex::new(answer_send)),
                        futures::future::FutureExt::shared(
                            futures::future::FutureExt::boxed(async move {
                                tokio::time::timeout(
                                    sig.config.timeout,
                                    Peer::new(
                                        sig,
                                        peer_url,
                                        peer_id,
                                        peer_uniq,
                                        ice_servers,
                                        new_peer_dir,
                                        peer_cmd_recv,
                                    ),
                                )
                                .await
                                .map_err(
                                    |_| {
                                        Error::str(
                                            "Timeout awaiting peer connection",
                                        )
                                    },
                                )?
                            }),
                        ),
                    )
                })
                .clone()
        };

        // make sure to drop this after releasing our mutex lock
        drop(tmp);

        fut.await
    }

    pub fn ban(&self, id: Id) {
        let r = self.peer_map.lock().unwrap().get(&id).cloned();
        if let Some((uniq, _, _, _)) = r {
            close_peer(&self.sig.weak_peer_map, id, uniq);
        }
    }
}

pub(crate) fn close_peer(
    weak_peer_map: &Weak<Mutex<PeerMap>>,
    peer_id: Id,
    close_peer_uniq: u64,
) {
    let mut tmp = None;

    if let Some(peer_map) = weak_peer_map.upgrade() {
        let mut lock = peer_map.lock().unwrap();
        if let Some((peer_uniq, cmd, ans, peer)) = lock.remove(&peer_id) {
            if close_peer_uniq != peer_uniq {
                // most of the time we'll be closing the real one,
                // so optimize for that case, and cause a hash probe
                // in the less likely case some race caused us to
                // try to remove the wrong one.
                tmp = lock.insert(peer_id, (peer_uniq, cmd, ans, peer));
            } else {
                tmp = Some((peer_uniq, cmd, ans, peer));
            }
        }
    }

    // make sure nothing is dropped while we're holding the mutex lock
    drop(tmp);
}
