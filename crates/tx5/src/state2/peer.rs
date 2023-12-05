use super::*;

pub(crate) struct Peer {
    pub peer_uniq: PeerUniq,
}

impl Peer {
    pub fn new(
        _assoc: &mut Assoc<'_, '_, '_>,
        _peer_id: Id,
        _is_polite: bool,
        _is_outgoing: bool,
    ) -> Self {
        let peer_uniq = uniq();

        Self {
            peer_uniq,
        }
    }

    pub fn cmd(&mut self, _assoc: &mut Assoc<'_, '_, '_>, cmd: PeerCmd) {
        match cmd {
            PeerCmd::SendMsg { permit: _, data: _, resp: _ } => {
                todo!()
            }
        }
    }
}
