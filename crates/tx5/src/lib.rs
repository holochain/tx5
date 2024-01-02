#![deny(missing_docs)]
#![deny(warnings)]
#![deny(unsafe_code)]
#![doc = tx5_core::__doc_header!()]
//! # tx5
//!
//! Tx5 - The main holochain tx5 webrtc networking crate.
//!
//! # WebRTC Backend Features
//!
//! Tx5 can be backed currently by 1 of 2 backend webrtc libraries.
//!
//! - <b><i>`*`DEFAULT`*`</i></b> `backend-go-pion` - The pion webrtc library
//!   writen in go (golang).
//!   - [https://github.com/pion/webrtc](https://github.com/pion/webrtc)
//! - `backend-webrtc-rs` - The rust webrtc library.
//!   - [https://github.com/webrtc-rs/webrtc](https://github.com/webrtc-rs/webrtc)
//!
//! The go pion library is currently the default as it is more mature
//! and well tested, but comes with some overhead of calling into a different
//! memory/runtime. When the rust library is stable enough for holochain's
//! needs, we will switch the default. To switch now, or if you want to
//! make sure the backend doesn't change out from under you, set
//! no-default-features and explicitly enable the backend of your choice.

#[cfg(any(
    not(any(feature = "backend-go-pion", feature = "backend-webrtc-rs")),
    all(feature = "backend-go-pion", feature = "backend-webrtc-rs"),
))]
compile_error!("Must specify exactly 1 webrtc backend");

/// Re-exported dependencies.
pub mod deps {
    pub use tx5_core;
    pub use tx5_core::deps::*;
    pub use tx5_signal;
    pub use tx5_signal::deps::*;
}

pub use tx5_core::{Error, ErrorExt, Id, Result, Tx5InitConfig, Tx5Url};

mod ep3;
pub use ep3::*;

mod back_buf;
pub use back_buf::*;

/// A set of distinct chunks of bytes that can be treated as a single unit.
#[derive(Default)]
pub struct BytesList(pub std::collections::VecDeque<bytes::Bytes>);

impl BytesList {
    /// Construct a new BytesList.
    pub fn new() -> Self {
        Self::default()
    }

    /// Clear the data.
    pub fn clear(&mut self) {
        self.0.clear();
    }

    /// Push a new bytes::Bytes into this BytesList.
    pub fn push(&mut self, data: bytes::Bytes) {
        if bytes::Buf::has_remaining(&data) {
            self.0.push_back(data);
        }
    }

    /// Convert into a trait object.
    pub fn into_dyn(self) -> Box<dyn bytes::Buf + 'static + Send> {
        Box::new(self)
    }

    /// Copy data into a `Vec<u8>`. You should avoid this if possible.
    pub fn to_vec(&self) -> Vec<u8> {
        use bytes::Buf;
        let mut out = Vec::with_capacity(self.remaining());
        for b in self.0.iter() {
            out.extend_from_slice(b);
        }
        out
    }
}

impl bytes::Buf for BytesList {
    fn remaining(&self) -> usize {
        self.0.iter().map(|b| b.remaining()).sum()
    }

    fn chunk(&self) -> &[u8] {
        match self.0.get(0) {
            Some(b) => b.chunk(),
            None => &[],
        }
    }

    #[allow(clippy::comparison_chain)] // clearer written explicitly
    fn advance(&mut self, mut cnt: usize) {
        loop {
            let mut item = match self.0.pop_front() {
                Some(item) => item,
                None => return,
            };

            let rem = item.remaining();
            if rem == cnt {
                return;
            } else if rem < cnt {
                cnt -= rem;
            } else if rem > cnt {
                item.advance(cnt);
                self.0.push_front(item);
                return;
            }
        }
    }
}
