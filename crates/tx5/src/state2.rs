#![allow(missing_docs)]
//! Tx5 v2 state machine.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use tx5_core::{Error, Id, Result, Tx5Url};

use crate::{BackBuf, Config2};

/// Signal server url.
pub type SigUrl = Tx5Url;

/// Peer Url.
pub type PeerUrl = Tx5Url;

/// Signal connection identifier.
pub type SigUniq = u64;

/// Peer connection identifier.
pub type PeerUniq = u64;

fn uniq() -> u64 {
    static UNIQ: std::sync::atomic::AtomicU64 =
        std::sync::atomic::AtomicU64::new(1);
    UNIQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Events output by tx5 v2 state machine.
pub enum State2Evt {
    /// Open a new signal server connection.
    SigCreate { sig_url: SigUrl, sig_uniq: SigUniq },

    /// Close a signal server connection.
    SigClose {
        sig_url: SigUrl,
        sig_uniq: SigUniq,
        err: Error,
    },

    /// Open a new peer connection.
    PeerCreate {
        sig_url: SigUrl,
        sig_uniq: SigUniq,
        peer_id: Id,
        peer_uniq: PeerUniq,
        ice_servers: Arc<serde_json::Value>,
    },

    /// Close a peer connection.
    PeerClose {
        peer_id: Id,
        peer_uniq: PeerUniq,
        err: Error,
    },
}

/// Input peer sub commands for the tx5 v2 state machine.
pub enum PeerCmd {}

/// Input sig sub commands for the tx5 v2 state machine.
pub enum SigCmd {
    /// Notify of a open and ready signal server connection.
    Open {
        this_id: Id,
        ice_servers: Arc<serde_json::Value>,
    },

    /// Make sure we have a peer connection.
    PeerAssert {
        peer_id: Id,
        is_polite: bool,
        is_outgoing: bool,
    },

    /// Close a peer connection.
    PeerClose {
        peer_id: Id,
        peer_uniq: PeerUniq,
        err: Error,
    },

    /// Peer sub command.
    PeerCmd {
        peer_id: Id,
        peer_uniq: PeerUniq,
        peer_cmd: PeerCmd,
    },
}

/// Input commands for the tx5 v2 state machine.
pub enum State2Cmd {
    /// Update progress / timers as needed.
    Tick,

    /// Make sure we have a signal server connection.
    SigAssert { sig_url: SigUrl, is_listening: bool },

    /// Close a signal server connection.
    SigClose {
        sig_url: SigUrl,
        sig_uniq: SigUniq,
        err: Error,
    },

    /// Signal sub command.
    SigCmd {
        sig_url: SigUrl,
        sig_uniq: SigUniq,
        sig_cmd: SigCmd,
    },

    /// Send a message.
    SendMsg {
        sig_url: SigUrl,
        peer_id: Id,
        permit: tokio::sync::OwnedSemaphorePermit,
        data: BackBuf,
        resp: Arc<Mutex<Option<tokio::sync::oneshot::Sender<Result<()>>>>>,
    },
}

/// Bundling associated references makes it easier to
/// work with rust's borrowing rules.
pub(crate) struct Assoc<'s, 'evt_list, 'want_tick> {
    pub config: &'s Config2,
    pub this_id: &'s Id,
    #[allow(dead_code)]
    pub cmd_list: &'s mut VecDeque<State2Cmd>,
    pub evt_list: &'evt_list mut VecDeque<State2Evt>,
    pub want_tick: &'want_tick mut bool,
}

mod peer;
use peer::*;

mod sig;
use sig::*;

#[derive(Default)]
struct SigMap(HashMap<SigUrl, Sig>);

impl SigMap {
    pub fn sig_assert(
        &mut self,
        assoc: &mut Assoc<'_, '_, '_>,
        sig_url: SigUrl,
        is_listening: bool,
    ) -> &mut Sig {
        let r = self
            .0
            .entry(sig_url.clone())
            .or_insert_with(|| Sig::new(assoc, sig_url));
        if is_listening {
            r.set_is_listening();
        }
        r
    }

    pub fn sig_close(
        &mut self,
        assoc: &mut Assoc<'_, '_, '_>,
        sig_url: SigUrl,
        sig_uniq: SigUniq,
        err: Error,
    ) {
        let mut remove = false;

        if let Some(sig) = self.0.get_mut(&sig_url) {
            if sig.sig_uniq != sig_uniq {
                return;
            }
            sig.close(assoc, err);

            if !sig.is_listening {
                remove = true;
            }
        }

        if remove {
            self.0.remove(&sig_url);
        }
    }

    pub fn tick(&mut self, assoc: &mut Assoc<'_, '_, '_>) {
        let mut rm = Vec::new();

        for (sig_url, sig) in self.0.iter_mut() {
            if let Err(err) = sig.tick(assoc) {
                rm.push((sig_url.clone(), sig.sig_uniq, err));
            }
        }

        for (sig_url, sig_uniq, err) in rm {
            self.sig_close(assoc, sig_url, sig_uniq, err.into());
        }
    }

    pub fn get_sig(
        &mut self,
        sig_url: &SigUrl,
        sig_uniq: SigUniq,
    ) -> Option<&mut Sig> {
        self.0.get_mut(sig_url).and_then(|sig| {
            if sig.sig_uniq == sig_uniq {
                Some(sig)
            } else {
                None
            }
        })
    }

    pub fn sig_cmd(
        &mut self,
        assoc: &mut Assoc<'_, '_, '_>,
        sig_url: SigUrl,
        sig_uniq: SigUniq,
        sig_cmd: SigCmd,
    ) {
        if let Some(sig) = self.get_sig(&sig_url, sig_uniq) {
            sig.cmd(assoc, sig_cmd);
        }
    }

    #[allow(unused_variables)]
    pub fn send_msg(
        &mut self,
        assoc: &mut Assoc<'_, '_, '_>,
        sig_url: SigUrl,
        peer_id: Id,
        permit: tokio::sync::OwnedSemaphorePermit,
        data: BackBuf,
        resp: Arc<Mutex<Option<tokio::sync::oneshot::Sender<Result<()>>>>>,
    ) {
        let is_polite = &peer_id > assoc.this_id;
        let sig = self.sig_assert(assoc, sig_url.clone(), false);
        let peer = sig.peer_assert(assoc, peer_id, is_polite, true);
        peer.send_msg(assoc, permit, data, resp);
    }
}

/// Tx5 v2 state machine.
pub struct State2 {
    config: Arc<Config2>,
    this_id: Id,
    cmd_list: VecDeque<State2Cmd>,
    sig_map: SigMap,
    last_tick: tokio::time::Instant,
}

impl State2 {
    pub fn new(config: Arc<Config2>, this_id: Id) -> Self {
        Self {
            config,
            this_id,
            cmd_list: VecDeque::new(),
            sig_map: SigMap::default(),
            last_tick: tokio::time::Instant::now(),
        }
    }

    pub fn cmd(&mut self, cmd: State2Cmd, evt_list: &mut VecDeque<State2Evt>) {
        self.cmd_list.push_back(cmd);

        let Self {
            config,
            this_id,
            cmd_list,
            sig_map,
            last_tick,
        } = self;

        let mut want_tick =
            last_tick.elapsed() >= std::time::Duration::from_secs(1);
        let want_tick = &mut want_tick;

        while let Some(cmd) = cmd_list.pop_front() {
            let mut assoc = Assoc {
                config,
                this_id,
                cmd_list,
                evt_list,
                want_tick,
            };

            match cmd {
                State2Cmd::Tick => (),
                State2Cmd::SigAssert {
                    sig_url,
                    is_listening,
                } => {
                    sig_map.sig_assert(&mut assoc, sig_url, is_listening);
                }
                State2Cmd::SigClose {
                    sig_url,
                    sig_uniq,
                    err,
                } => sig_map.sig_close(&mut assoc, sig_url, sig_uniq, err),
                State2Cmd::SigCmd {
                    sig_url,
                    sig_uniq,
                    sig_cmd,
                } => sig_map.sig_cmd(&mut assoc, sig_url, sig_uniq, sig_cmd),
                State2Cmd::SendMsg {
                    sig_url,
                    peer_id,
                    permit,
                    data,
                    resp,
                } => sig_map
                    .send_msg(&mut assoc, sig_url, peer_id, permit, data, resp),
            }
        }

        if *want_tick {
            sig_map.tick(&mut Assoc {
                config,
                this_id,
                cmd_list,
                evt_list,
                want_tick,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state2() {
        let this_id = Id([0; 32]);
        let peer_id = Id([1; 32]);

        let mut s = State2::new(Arc::new(Config2::default()), this_id);
        let mut evt = VecDeque::new();
        let limit = Arc::new(tokio::sync::Semaphore::new(
            tokio::sync::Semaphore::MAX_PERMITS,
        ));

        let sig_url = Tx5Url::new("wss://bla").unwrap();

        s.cmd(
            State2Cmd::SigAssert {
                sig_url: sig_url.clone(),
                is_listening: true,
            },
            &mut evt,
        );

        let permit = limit.clone().try_acquire_owned().unwrap();

        let (resp, _r) = tokio::sync::oneshot::channel();

        let resp = Arc::new(Mutex::new(Some(resp)));

        s.cmd(
            State2Cmd::SendMsg {
                sig_url: sig_url.clone(),
                peer_id,
                permit,
                data: BackBuf::from_slice(b"hello").unwrap(),
                resp,
            },
            &mut evt,
        );
    }
}
