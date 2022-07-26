use crate::*;
use parking_lot::Mutex;
use std::collections::hash_map;
use std::collections::HashMap;
use tx4_signal_core::Id;

const MAX_CON: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PeerId {
    pub rem_id: Arc<Id>,
    pub rem_pk: Arc<Id>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum PeerState {
    /// We have discovered this id, but haven't yet spoken
    New,

    /// We are in the process of establishing a connection
    Connect,

    /// We have previously spoken successfully with this peer
    Done,

    /// We had an error speaking to this peer, pause trying again
    Block,
}

#[derive(Debug)]
pub struct PeerInfo {
    pub id: PeerId,
    pub state: PeerState,
    pub last_touch: std::time::Instant,
    pub friendly_name: Option<String>,
    pub shoutout: Option<String>,
}

#[derive(Clone)]
pub struct State(Arc<Mutex<StateInner>>);

impl std::fmt::Debug for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let inner = self.0.lock();
        let map_len = inner.map.len();
        f.debug_struct("State").field("map_len", &map_len).finish()
    }
}

impl State {
    pub fn new(friendly_name: String, shoutout: String) -> Self {
        let loc_id = Id::from_slice(&[0; 32]).unwrap();
        let loc_pk = Id::from_slice(&[0; 32]).unwrap();
        Self(Arc::new(Mutex::new(StateInner::new(
            loc_id,
            loc_pk,
            friendly_name,
            shoutout,
        ))))
    }

    pub fn set_loc(&self, loc_id: Arc<Id>, loc_pk: Arc<Id>) {
        let mut inner = self.0.lock();
        inner.loc_id = loc_id;
        inner.loc_pk = loc_pk;
    }

    pub fn discover(&self, rem_id: Arc<Id>, rem_pk: Arc<Id>) {
        let mut inner = self.0.lock();

        if rem_id == inner.loc_id && rem_pk == inner.loc_pk {
            return;
        }

        if let hash_map::Entry::Vacant(e) = inner.map.entry(PeerId {
            rem_id: rem_id.clone(),
            rem_pk: rem_pk.clone(),
        }) {
            let id = PeerId { rem_id, rem_pk };
            let info = PeerInfo {
                id,
                state: PeerState::New,
                last_touch: std::time::Instant::now(),
                friendly_name: None,
                shoutout: None,
            };
            tracing::debug!(?info, "Discover");
            e.insert(info);
        }
    }

    pub fn contact(&self, id: PeerId, friendly_name: String, shoutout: String) {
        let mut inner = self.0.lock();

        let now = std::time::Instant::now();

        let mut r = inner
            .map
            .entry(id.clone())
            .or_insert_with(move || PeerInfo {
                id,
                state: PeerState::New,
                last_touch: now,
                friendly_name: None,
                shoutout: None,
            });

        r.friendly_name = Some(friendly_name);
        r.shoutout = Some(shoutout);
        r.last_touch = now;

        tracing::info!(?r, "CONTACT");
    }

    pub fn con_done(&self, id: PeerId, should_block: bool) {
        let mut inner = self.0.lock();

        // randomize slightly
        use rand::Rng;
        //let secs = rand::thread_rng().gen_range(0..20);
        let secs = rand::thread_rng().gen_range(0..5);
        let now = std::time::Instant::now() + std::time::Duration::from_secs(secs);

        let state = if should_block {
            PeerState::Block
        } else {
            PeerState::Done
        };

        let mut r = inner
            .map
            .entry(id.clone())
            .or_insert_with(move || PeerInfo {
                id,
                state,
                last_touch: now,
                friendly_name: None,
                shoutout: None,
            });

        r.last_touch = now;
        r.state = state;

        tracing::debug!(?r, "DropCon");
    }

    pub fn check_want_outgoing(&self) -> Vec<PeerId> {
        let mut out = Vec::new();

        let mut inner = self.0.lock();

        let mut con_count = inner
            .map
            .iter()
            .filter(|(_, i)| i.state == PeerState::Connect)
            .count();

        let mut remove = Vec::new();

        {
            let now = std::time::Instant::now();
            let mut items = inner.map.iter_mut().collect::<Vec<_>>();
            items.sort_unstable_by(|a, b| {
                let a = now - a.1.last_touch;
                let b = now - b.1.last_touch;

                // handle oldest items first
                a.cmp(&b).reverse()
            });

            for (_, i) in items {
                if i.state == PeerState::Block && i.last_touch.elapsed().as_secs() > 60 * 5 {
                    remove.push(i.id.clone());
                    continue;
                }

                //if i.state == PeerState::Done && i.last_touch.elapsed().as_secs() > 60 {
                if i.state == PeerState::Done && i.last_touch.elapsed().as_secs() > 10 {
                    i.state = PeerState::New;
                }

                if con_count >= MAX_CON {
                    continue;
                }

                if i.state == PeerState::New {
                    i.state = PeerState::Connect;
                    out.push(i.id.clone());
                    con_count += 1;
                }
            }
        }

        for remove in remove {
            inner.map.remove(&remove);
        }

        out
    }

    pub fn check_want_incoming(&self, id: PeerId) -> bool {
        let mut inner = self.0.lock();

        let now = std::time::Instant::now();

        let mut r = inner
            .map
            .entry(id.clone())
            .or_insert_with(move || PeerInfo {
                id,
                state: PeerState::New,
                last_touch: now,
                friendly_name: None,
                shoutout: None,
            });

        if r.state != PeerState::New && r.state != PeerState::Done {
            return false;
        }

        r.state = PeerState::Connect;

        true
    }

    pub fn gen_msg(&self) -> Result<tx4_go_pion::GoBuf> {
        let mut out = tx4_go_pion::GoBuf::new().map_err(other_err)?;
        let inner = self.0.lock();
        let mut buf = [0; 32];
        buf[0..inner.friendly_name.as_bytes().len()]
            .copy_from_slice(inner.friendly_name.as_bytes());
        out.extend(&buf[..]).map_err(other_err)?;
        buf[..].fill(0);
        buf[0..inner.shoutout.as_bytes().len()].copy_from_slice(inner.shoutout.as_bytes());
        out.extend(&buf[..]).map_err(other_err)?;
        for (_id, info) in inner.map.iter() {
            if info.state != PeerState::Block {
                out.extend(&*info.id.rem_id).map_err(other_err)?;
                out.extend(&*info.id.rem_pk).map_err(other_err)?;
            }
        }
        Ok(out)
    }
}

#[derive(Debug)]
struct StateInner {
    loc_id: Arc<Id>,
    loc_pk: Arc<Id>,
    friendly_name: String,
    shoutout: String,
    map: HashMap<PeerId, PeerInfo>,
}

impl StateInner {
    pub fn new(loc_id: Arc<Id>, loc_pk: Arc<Id>, friendly_name: String, shoutout: String) -> Self {
        Self {
            loc_id,
            loc_pk,
            friendly_name,
            shoutout,
            map: HashMap::new(),
        }
    }
}
