#![allow(dead_code)]

use super::*;

use std::collections::{hash_map, HashMap};

fn uniq() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static UNIQ: AtomicU64 = AtomicU64::new(0);
    UNIQ.fetch_add(1, Ordering::Relaxed)
}

struct SigStateData {
    key: u64,
    init_ok: bool,
    init_cb_list: Vec<tokio::sync::oneshot::Sender<Result<()>>>,
    sig_evt: tokio::sync::mpsc::UnboundedSender<Result<SigStateEvt>>,
}

impl SigStateData {
    pub fn new(
        respond: tokio::sync::oneshot::Sender<Result<()>>,
    ) -> (Self, u64, ManyRcv<Result<SigStateEvt>>) {
        let (sig_evt_send, sig_evt_recv) =
            tokio::sync::mpsc::unbounded_channel();
        let key = uniq();
        (
            Self {
                key,
                init_ok: false,
                init_cb_list: vec![respond],
                sig_evt: sig_evt_send,
            },
            key,
            ManyRcv(sig_evt_recv),
        )
    }
}

struct StateDataInner {
    evt: tokio::sync::mpsc::UnboundedSender<Result<StateEvt>>,
    signal_map: HashMap<Tx4Url, SigStateData>,
}

impl StateDataInner {
    pub fn new(
        evt: tokio::sync::mpsc::UnboundedSender<Result<StateEvt>>,
    ) -> Self {
        Self {
            evt,
            signal_map: HashMap::new(),
        }
    }
}

#[derive(Clone)]
pub struct StateData(Store<StateDataInner>);

impl StateData {
    pub fn new(
        evt: tokio::sync::mpsc::UnboundedSender<Result<StateEvt>>,
    ) -> Self {
        Self(Store::new(StateDataInner::new(evt)))
    }

    pub fn shutdown_full(&self, maybe_err: Option<std::io::Error>) {
        let _ = self.0.access_mut(move |inner| {
            inner.close();
            if let Some(err) = maybe_err {
                for (_, sig) in inner.signal_map.iter_mut() {
                    let _ = sig.sig_evt.send(Err(err.err_clone()));
                }
                let _ = inner.evt.send(Err(err));
            }
            Ok(())
        });
    }

    pub fn new_listener_sig_ok(&self, key: u64, url: &Tx4Url) {
        let _ = self.0.access_mut(move |inner| {
            if let Some(mut sig) = inner.signal_map.get_mut(url) {
                if sig.key != key {
                    // whoops! that wasn't us!
                    return Ok(());
                }
                sig.init_ok = true;
                let resp_list = std::mem::take(&mut sig.init_cb_list);
                inner.defer(move |_| {
                    for resp in resp_list {
                        let _ = resp.send(Ok(()));
                    }
                });
            }
            Ok(())
        });
    }

    pub fn new_listener_sig_err(
        &self,
        key: u64,
        url: &Tx4Url,
        err: std::io::Error,
    ) {
        let _ = self.0.access_mut(move |inner| {
            if let Some(mut sig) = inner.signal_map.remove(url) {
                if sig.key != key {
                    // whoops! that wasn't us!
                    inner.signal_map.insert(url.clone(), sig);
                    return Ok(());
                }
                let resp_list = std::mem::take(&mut sig.init_cb_list);
                inner.defer(move |_| {
                    for resp in resp_list {
                        let _ = resp.send(Err(err.err_clone()));
                    }
                });
            }
            Ok(())
        });
    }

    pub fn check_new_listener_sig(
        &self,
        url: Tx4Url,
        respond: tokio::sync::oneshot::Sender<Result<()>>,
    ) -> Result<()> {
        let this = self.clone();
        self.0.access_mut(move |inner| {
            match inner.signal_map.entry(url.clone()) {
                hash_map::Entry::Occupied(mut e) => {
                    if e.get().init_ok {
                        inner.defer(move |_| {
                            let _ = respond.send(Ok(()));
                        });
                    } else {
                        e.get_mut().init_cb_list.push(respond);
                    }
                    Ok(())
                }
                hash_map::Entry::Vacant(e) => {
                    let (sig_state_data, key, sig_evt) =
                        SigStateData::new(respond);
                    e.insert(sig_state_data);
                    let seed = SigStateSeed {
                        done: false,
                        key,
                        url: url.clone(),
                        state_data: this,
                        sig_evt: Some(sig_evt),
                    };
                    let _ = inner.evt.send(Ok(StateEvt::NewSig(seed)));
                    Ok(())
                }
            }
        })
    }
}
