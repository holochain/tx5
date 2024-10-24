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

pub use tx5_core::Tx5InitConfig;

macro_rules! breakable_timeout {
    ($($t:tt)*) => {
        tokio::time::timeout(
            ::tx5_core::Tx5InitConfig::get().slow_app_timeout,
            async {
                loop {
                    {$($t)*}
                    break;
                }
                std::io::Result::Ok(())
            }
        ).await
    };
}

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

struct CloseRecv<T: 'static + Send>(futures::channel::mpsc::Receiver<T>);

impl<T: 'static + Send> CloseRecv<T> {
    pub async fn recv(&mut self) -> Option<T> {
        use futures::stream::StreamExt;
        self.0.next().await
    }
}

struct CloseSend<T: 'static + Send> {
    sender: Arc<Mutex<Option<futures::channel::mpsc::Sender<T>>>>,
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
            let s = self.sender.lock().unwrap().take();
            if let Some(mut s) = s {
                s.close_channel();
            }
        }
    }
}

impl<T: 'static + Send> CloseSend<T> {
    pub fn channel() -> (Self, CloseRecv<T>) {
        Self::sized_channel(32)
    }

    pub fn sized_channel(size: usize) -> (Self, CloseRecv<T>) {
        let (s, r) = futures::channel::mpsc::channel(size);
        (
            Self {
                sender: Arc::new(Mutex::new(Some(s))),
                close_on_drop: false,
            },
            CloseRecv(r),
        )
    }

    pub fn set_close_on_drop(&mut self, close_on_drop: bool) {
        self.close_on_drop = close_on_drop;
    }

    #[allow(dead_code)] // only used in libdatachannel backend
    pub fn send_or_close(&self, t: T) {
        let mut lock = self.sender.lock().unwrap();
        if let Some(sender) = &mut *lock {
            if sender.try_send(t).is_err() {
                sender.close_channel();
                *lock = None;
            }
        }
    }

    pub fn send(
        &self,
        t: T,
    ) -> impl Future<Output = Result<()>> + 'static + Send {
        use futures::sink::SinkExt;
        let s = self.sender.lock().unwrap().clone();
        async move {
            match s {
                Some(mut s) => {
                    s.send(t).await.map_err(|_| ErrorKind::BrokenPipe.into())
                }
                None => Err(ErrorKind::BrokenPipe.into()),
            }
        }
    }

    #[allow(dead_code)] // only used in go_pion backend
    pub fn send_slow_app(
        &self,
        t: T,
    ) -> impl Future<Output = Result<()>> + 'static + Send {
        use futures::sink::SinkExt;

        let s = self.sender.lock().unwrap().clone();
        async move {
            match s {
                Some(mut s) => {
                    match tokio::time::timeout(
                        tx5_core::Tx5InitConfig::get().slow_app_timeout,
                        s.send(t),
                    )
                    .await
                    {
                        Err(_) => {
                            tracing::warn!(
                                "Closing connection due to slow app"
                            );
                            Err(ErrorKind::TimedOut.into())
                        }
                        Ok(Err(_)) => Err(ErrorKind::BrokenPipe.into()),
                        Ok(Ok(_)) => Ok(()),
                    }
                }
                None => Err(ErrorKind::BrokenPipe.into()),
            }
        }
    }
}

mod config;
pub use config::*;

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
