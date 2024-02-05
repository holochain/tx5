use super::*;

type PeerCmdSend = EventSend<PeerCmd>;
type AnswerSend =
    Arc<Mutex<Option<tokio::sync::oneshot::Sender<serde_json::Value>>>>;
pub(crate) type PeerMap = HashMap<
    Id,
    (
        u64,
        PeerCmdSend,
        AnswerSend,
        AbortableTimedSharedFuture<Arc<Peer>>,
    ),
>;

pub(crate) struct SigShared {
    pub ep: Arc<EpShared>,
    pub weak_sig: Weak<Sig>,
    pub sig_uniq: u64,
    pub weak_peer_map: Weak<Mutex<PeerMap>>,
}

impl std::ops::Deref for SigShared {
    type Target = Arc<EpShared>;

    fn deref(&self) -> &Self::Target {
        &self.ep
    }
}

pub(crate) struct SigDrop {
    pub ep_uniq: u64,
    pub sig_uniq: u64,
    pub sig_url: SigUrl,
    pub weak_sig_map: Weak<Mutex<SigMap>>,
}

impl Drop for SigDrop {
    fn drop(&mut self) {
        tracing::info!(%self.ep_uniq, %self.sig_uniq, %self.sig_url, "Signal Connection Close");

        close_sig(&self.weak_sig_map, &self.sig_url, self.sig_uniq);
    }
}

pub(crate) struct Sig {
    _sig_drop: SigDrop,
    sig: Arc<SigShared>,
    _permit: tokio::sync::OwnedSemaphorePermit,
    recv_task: tokio::task::JoinHandle<()>,
    ice_servers: Arc<serde_json::Value>,
    peer_map: Arc<Mutex<PeerMap>>,
    sig_cli: tx5_signal::Cli,
}

impl Drop for Sig {
    fn drop(&mut self) {
        self.sig.metric_conn_count.add(-1);
        self.recv_task.abort();
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
        _sig_drop: SigDrop,
        ep: Arc<EpShared>,
        sig_uniq: u64,
        sig_url: SigUrl,
    ) -> CRes<Arc<Self>> {
        ep.metric_conn_count.add(1);

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
                _sig_drop,
                sig: Arc::new(SigShared {
                    ep,
                    weak_sig: weak_sig.clone(),
                    sig_uniq,
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

        let (peer_uniq, _peer_cmd_send, _answer_send, fut) = {
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
                    let (peer_cmd_send, peer_cmd_recv) =
                        EventSend::new(sig.config.internal_event_channel_size);
                    let _peer_drop = PeerDrop {
                        ep_uniq: sig.ep_uniq,
                        sig_uniq: sig.sig_uniq,
                        peer_uniq,
                        peer_id,
                        weak_peer_map: sig.weak_peer_map.clone(),
                    };
                    (
                        peer_uniq,
                        peer_cmd_send,
                        Arc::new(Mutex::new(answer_send)),
                        AbortableTimedSharedFuture::new(
                            sig.config.timeout,
                            Error::str("Timeout awaiting peer connection")
                                .into(),
                            Peer::new(
                                _peer_drop,
                                sig,
                                peer_url,
                                peer_id,
                                peer_uniq,
                                ice_servers,
                                new_peer_dir,
                                peer_cmd_recv,
                            ),
                        ),
                    )
                })
                .clone()
        };

        // make sure to drop this after releasing our mutex lock
        if let Some((_peer_uniq, _cmd, _ans, peer_fut)) = tmp {
            peer_fut.abort(Error::str("Dropping connection because we are the polite node and received an offer from the remote").into());
            drop(peer_fut);
        }

        match fut.await {
            Err(err) => {
                // if a new peer got added in the mean time, return that instead
                let r = self.peer_map.lock().unwrap().get(&peer_id).cloned();

                if let Some((new_peer_uniq, _cmd, _ans, new_peer_fut)) = r {
                    if new_peer_uniq != peer_uniq {
                        return new_peer_fut.await;
                    }
                }

                Err(err)
            }
            Ok(r) => Ok(r),
        }
    }

    pub fn ban(&self, id: Id) {
        let r = self.peer_map.lock().unwrap().get(&id).cloned();
        if let Some((uniq, _, _, _)) = r {
            close_peer(&self.sig.weak_peer_map, id, uniq);
        }
    }

    pub async fn broadcast(&self, data: &[u8]) {
        let mut task_list = Vec::new();

        let fut_list = self
            .peer_map
            .lock()
            .unwrap()
            .values()
            .map(|v| v.3.clone())
            .collect::<Vec<_>>();

        for fut in fut_list {
            task_list.push(async move {
                // timeouts are built into this future as well
                // as the peer.send function
                if let Ok(peer) = fut.await {
                    let _ = peer.send(data).await;
                }
            });
        }

        futures::future::join_all(task_list).await;
    }

    pub async fn get_stats(&self) -> Vec<(Id, serde_json::Value)> {
        let mut task_list = Vec::new();

        let fut_list = self
            .peer_map
            .lock()
            .unwrap()
            .iter()
            .map(|(k, v)| (*k, v.3.clone()))
            .collect::<Vec<_>>();

        for (peer_id, fut) in fut_list {
            task_list.push(async move {
                if let Ok(peer) = fut.await {
                    match peer.stats().await {
                        Ok(s) => Some((peer_id, s)),
                        _ => None,
                    }
                } else {
                    None
                }
            })
        }

        futures::future::join_all(task_list)
            .await
            .into_iter()
            .flatten()
            .collect()
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
    if let Some((_peer_uniq, _cmd, _ans, peer_fut)) = tmp {
        peer_fut.abort(Error::id("Close").into());
        drop(peer_fut);
    }
}
