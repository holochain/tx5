use crate::{state2::State2, Config2};

use tx5_core::Id;

/// Tx5 v2 endpoint.
pub struct Ep2 {
}

impl Ep2 {
    /// Construct a new tx5 v2 endpoint.
    pub fn new(config: Config2) -> Self {
        let _s = State2::new(config, Id([0; 32]));

        Self {
        }
    }
}
