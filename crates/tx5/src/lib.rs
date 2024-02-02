#![deny(missing_docs)]
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

pub(crate) mod back_buf;
pub(crate) use back_buf::*;

pub(crate) mod proto;

/// Make a shared (clonable) future abortable and set a timeout.
/// The timeout is managed by tokio::time::timeout.
/// The clone-ability is managed by futures::future::shared.
/// The abortability is NOT managed by futures::future::abortable,
/// because we need to be able to pass in a specific error when aborting,
/// so it is managed via a tokio::sync::oneshot channel and tokio::select!.
#[derive(Clone)]
struct AbortableTimedSharedFuture<T: Clone> {
    f: futures::future::Shared<
        futures::future::BoxFuture<'static, std::result::Result<T, Error>>,
    >,
    a: std::sync::Arc<
        std::sync::Mutex<Option<tokio::sync::oneshot::Sender<Error>>>,
    >,
}

impl<T: Clone> AbortableTimedSharedFuture<T> {
    /// Construct a new AbortableTimedSharedFuture that will timeout
    /// after the given duration.
    pub fn new<F>(
        timeout: std::time::Duration,
        timeout_err: Error,
        f: F,
    ) -> Self
    where
        F: std::future::Future<Output = std::result::Result<T, Error>>
            + 'static
            + Send,
    {
        let (a, ar) = tokio::sync::oneshot::channel();
        let a = std::sync::Arc::new(std::sync::Mutex::new(Some(a)));
        Self {
            f: futures::future::FutureExt::shared(
                futures::future::FutureExt::boxed(async move {
                    tokio::time::timeout(
                        timeout,
                        async move {
                            tokio::select! {
                                r = async {
                                    Err(ar.await.map_err(|_| Error::id("AbortHandleDropped"))?)
                                } => r,
                                r = f => r,
                            }
                        },
                    )
                    .await
                    .map_err(|_| timeout_err)?
                }),
            ),
            a,
        }
    }

    /// Abort this future with the given error.
    pub fn abort(&self, err: Error) {
        let a = self.a.lock().unwrap().take();
        if let Some(a) = a {
            let _ = a.send(err);
        }
    }
}

impl<T: Clone> std::future::Future for AbortableTimedSharedFuture<T> {
    type Output = std::result::Result<T, Error>;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        std::pin::Pin::new(&mut self.f).poll(cx)
    }
}

#[cfg(test)]
mod test_behavior;

#[cfg(test)]
mod test {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn atsf_traits() {
        fn check<F>(_f: F)
        where
            F: Send + Sync + Unpin,
        {
        }

        let a = AbortableTimedSharedFuture::new(
            std::time::Duration::from_millis(10),
            Error::str("my timeout err").into(),
            async move { Ok(()) },
        );

        check(a);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn atsf_happy() {
        AbortableTimedSharedFuture::new(
            std::time::Duration::from_secs(1),
            Error::id("to").into(),
            async move { Ok(()) },
        )
        .await
        .unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn atsf_timeout() {
        let r = AbortableTimedSharedFuture::new(
            std::time::Duration::from_millis(1),
            Error::id("to").into(),
            async move {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                Ok(())
            },
        )
        .await;
        assert_eq!("to", r.unwrap_err().to_string());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn atsf_abort() {
        let a = AbortableTimedSharedFuture::new(
            std::time::Duration::from_secs(1),
            Error::id("to").into(),
            async move {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                Ok(())
            },
        );
        {
            let a = a.clone();
            tokio::task::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                a.abort(Error::id("abort").into());
            });
        }
        let r = a.await;
        assert_eq!("abort", r.unwrap_err().to_string());
    }
}
