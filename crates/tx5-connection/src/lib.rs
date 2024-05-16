#![deny(missing_docs)]
#![deny(unsafe_code)]
#![doc = tx5_core::__doc_header!()]
//! # tx5-connection
//!
//! Holochain webrtc connection.
//! Starts by sending messages over the sbd signal server, if we can
//! upgrade to a proper webrtc p2p connection, we do so.
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

#[cfg(feature = "backend-go-pion")]
pub use tx5_go_pion::Tx5InitConfig;

use std::collections::HashMap;
use std::future::Future;
use std::io::{Error, ErrorKind, Result};
use std::sync::{Arc, Mutex, Weak};

pub use tx5_signal;
use tx5_signal::PubKey;

struct AbortTask<R>(tokio::task::JoinHandle<R>);

impl<R> Drop for AbortTask<R> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

struct CloseSend<T: 'static + Send> {
    sender: Arc<Mutex<Option<tokio::sync::mpsc::Sender<T>>>>,
    close_on_drop: bool,
}

impl<T: 'static + Send> Clone for CloseSend<T> {
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(),
            close_on_drop: false,
        }
    }
}

impl<T: 'static + Send> Drop for CloseSend<T> {
    fn drop(&mut self) {
        if self.close_on_drop {
            self.sender.lock().unwrap().take();
        }
    }
}

impl<T: 'static + Send> CloseSend<T> {
    pub fn channel() -> (Self, tokio::sync::mpsc::Receiver<T>) {
        let (s, r) = tokio::sync::mpsc::channel(32);
        (
            Self {
                sender: Arc::new(Mutex::new(Some(s))),
                close_on_drop: false,
            },
            r,
        )
    }

    pub fn set_close_on_drop(&mut self, close_on_drop: bool) {
        self.close_on_drop = close_on_drop;
    }

    pub fn send(
        &self,
        t: T,
    ) -> impl Future<Output = Result<()>> + 'static + Send {
        let s = self.sender.lock().unwrap().clone();
        async move {
            match s {
                Some(s) => {
                    s.send(t).await.map_err(|_| ErrorKind::BrokenPipe.into())
                }
                None => Err(ErrorKind::BrokenPipe.into()),
            }
        }
    }

    pub fn send_slow_app(
        &self,
        t: T,
    ) -> impl Future<Output = Result<()>> + 'static + Send {
        // Grace time to allow a slow app to catch up before we close a
        // connection to prevent our memory from filling up with backlogged
        // message data.
        const SLOW_APP_TO: std::time::Duration =
            std::time::Duration::from_millis(99);

        let s = self.sender.lock().unwrap().clone();
        async move {
            match s {
                Some(s) => match s.send_timeout(t, SLOW_APP_TO).await {
                    Err(
                        tokio::sync::mpsc::error::SendTimeoutError::Timeout(_),
                    ) => {
                        tracing::warn!("Closing connection due to slow app");
                        Err(ErrorKind::TimedOut.into())
                    }
                    Err(_) => Err(ErrorKind::BrokenPipe.into()),
                    Ok(_) => Ok(()),
                },
                None => Err(ErrorKind::BrokenPipe.into()),
            }
        }
    }
}

mod webrtc;

mod hub;
pub use hub::*;

mod conn;
pub use conn::*;

mod proto;

mod framed;
pub use framed::*;

#[cfg(test)]
mod test;
