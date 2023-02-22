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

use deps::{serde, serde_json};

use tx5_core::Uniq;
pub use tx5_core::{Error, ErrorExt, Id, Result, Tx5Url};

pub mod actor;

mod buf;
pub use buf::*;

/// Helper extension trait for `Box<dyn bytes::Buf + 'static + Send>`.
pub trait BytesBufExt {
    /// Convert into a `Vec<u8>`.
    fn to_vec(self) -> Result<Vec<u8>>;
}

impl BytesBufExt for Box<dyn bytes::Buf + 'static + Send> {
    fn to_vec(self) -> Result<Vec<u8>> {
        use bytes::Buf;
        use std::io::Read;
        let mut out = Vec::with_capacity(self.remaining());
        self.reader().read_to_end(&mut out)?;
        Ok(out)
    }
}

/// A set of distinct chunks of bytes that can be treated as a single unit
//#[derive(Clone, Default)]
#[derive(Default)]
struct BytesList(pub std::collections::VecDeque<bytes::Bytes>);

impl BytesList {
    /// Construct a new BytesList.
    pub fn new() -> Self {
        Self::default()
    }

    /*
        /// Construct a new BytesList with given capacity.
        pub fn with_capacity(capacity: usize) -> Self {
            Self(std::collections::VecDeque::with_capacity(capacity))
        }
    */

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

    /*
        /// Copy data into a Vec<u8>. You should avoid this if possible.
        pub fn to_vec(&self) -> Vec<u8> {
            use std::io::Read;
            let mut out = Vec::with_capacity(self.remaining());
            // data is local, it can't error, safe to unwrap
            self.clone().reader().read_to_end(&mut out).unwrap();
            out
        }

        /// Extract the contents of this BytesList into a new one
        pub fn extract(&mut self) -> Self {
            Self(std::mem::take(&mut self.0))
        }

        /// Remove specified byte cnt from the front of this list as a new list.
        /// Panics if self doesn't contain enough bytes.
        #[allow(clippy::comparison_chain)] // clearer written explicitly
        pub fn take_front(&mut self, mut cnt: usize) -> Self {
            let mut out = BytesList::new();
            loop {
                let mut item = match self.0.pop_front() {
                    Some(item) => item,
                    None => panic!("UnexpectedEof"),
                };

                let rem = item.remaining();
                if rem == cnt {
                    out.push(item);
                    return out;
                } else if rem < cnt {
                    out.push(item);
                    cnt -= rem;
                } else if rem > cnt {
                    out.push(item.split_to(cnt));
                    self.0.push_front(item);
                    return out;
                }
            }
        }
    */
}

/*
impl From<std::collections::VecDeque<bytes::Bytes>> for BytesList {
    #[inline(always)]
    fn from(v: std::collections::VecDeque<bytes::Bytes>) -> Self {
        Self(v)
    }
}

impl From<bytes::Bytes> for BytesList {
    #[inline(always)]
    fn from(b: bytes::Bytes) -> Self {
        let mut out = std::collections::VecDeque::with_capacity(8);
        out.push_back(b);
        out.into()
    }
}

impl From<Vec<u8>> for BytesList {
    #[inline(always)]
    fn from(v: Vec<u8>) -> Self {
        bytes::Bytes::from(v).into()
    }
}

impl From<&[u8]> for BytesList {
    #[inline(always)]
    fn from(b: &[u8]) -> Self {
        bytes::Bytes::copy_from_slice(b).into()
    }
}

impl<const N: usize> From<&[u8; N]> for BytesList {
    #[inline(always)]
    fn from(b: &[u8; N]) -> Self {
        bytes::Bytes::copy_from_slice(&b[..]).into()
    }
}
*/

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

pub mod state;

mod config;
pub use config::*;

mod endpoint;
pub use endpoint::*;

#[cfg(test)]
mod test;
