//! Quick-n-dirty actor system for the tx4 state.

use crate::*;
use parking_lot::Mutex;
use std::sync::{Arc, Weak};

fn uniq() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static UNIQ: AtomicU64 = AtomicU64::new(0);
    UNIQ.fetch_add(1, Ordering::Relaxed)
}

type BoxFut<'lt, T> =
    std::pin::Pin<Box<dyn std::future::Future<Output = T> + 'lt + Send>>;

type Logic<T> =
    Box<dyn FnOnce(T) -> BoxFut<'static, Option<T>> + 'static + Send>;

type Snd<T> = tokio::sync::mpsc::UnboundedSender<Logic<T>>;

/// Weak reference to an actor that doesn't inc the ref count.
pub struct ActorWeak<T: 'static + Send>(Weak<Mutex<Option<Snd<T>>>>, u64);

impl<T: 'static + Send> Clone for ActorWeak<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone(), self.1)
    }
}

impl<T: 'static + Send> PartialEq for ActorWeak<T> {
    fn eq(&self, rhs: &Self) -> bool {
        self.1 == rhs.1
    }
}

impl<T: 'static + Send> PartialEq<Actor<T>> for ActorWeak<T> {
    fn eq(&self, rhs: &Actor<T>) -> bool {
        self.1 == rhs.1
    }
}

impl<T: 'static + Send> Eq for ActorWeak<T> {}

impl<T: 'static + Send> ActorWeak<T> {
    /// Attempt to upgrade into a full actor.
    #[inline]
    pub fn upgrade(&self) -> Option<Actor<T>> {
        self.0.upgrade().map(|i| Actor(i, self.1))
    }
}

/// An actor managing state that can execute logic on that state.
pub struct Actor<T: 'static + Send>(Arc<Mutex<Option<Snd<T>>>>, u64);

impl<T: 'static + Send> Clone for Actor<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone(), self.1)
    }
}

impl<T: 'static + Send> PartialEq for Actor<T> {
    fn eq(&self, rhs: &Self) -> bool {
        self.1 == rhs.1
    }
}

impl<T: 'static + Send> PartialEq<ActorWeak<T>> for Actor<T> {
    fn eq(&self, rhs: &ActorWeak<T>) -> bool {
        self.1 == rhs.1
    }
}

impl<T: 'static + Send> Eq for Actor<T> {}

impl<T: 'static + Send> Actor<T> {
    /// Construct a new actor with initial state.
    pub fn new<Cb>(cb: Cb) -> Self
    where
        Cb: FnOnce(ActorWeak<T>) -> T,
    {
        let id = uniq();
        let (s, mut r) = tokio::sync::mpsc::unbounded_channel::<Logic<T>>();
        let actor = Self(Arc::new(Mutex::new(Some(s))), id);
        let mut t = cb(actor.weak());
        tokio::task::spawn(async move {
            while let Some(l) = r.recv().await {
                t = match l(t).await {
                    Some(t) => t,
                    None => break,
                }
            }
        });
        actor
    }

    /// Get a weak reference to this actor that won't bump the ref count.
    #[inline]
    pub fn weak(&self) -> ActorWeak<T> {
        ActorWeak(Arc::downgrade(&self.0), self.1)
    }

    /// Returns true if this actor been closed, either explicitly or
    /// due to a return of `None` on an exec call.
    #[inline]
    pub fn is_closed(&self) -> bool {
        self.0.lock().is_none()
    }

    /// Close this actor explicitly.
    pub fn close(&self) {
        let sender = self.0.lock().take();
        if let Some(sender) = sender {
            let _ = sender.send(Box::new(|_| Box::pin(async move { None })));
        }
    }

    /// Execute logic on the state maintained by this actor.
    #[inline]
    pub fn exec<R, Fut, Cb>(
        &self,
        cb: Cb,
    ) -> impl std::future::Future<Output = Result<R>> + 'static + Send
    where
        R: 'static + Send,
        Fut: std::future::Future<Output = (Option<T>, Result<R>)>
            + 'static
            + Send,
        Cb: FnOnce(T) -> Fut + 'static + Send,
    {
        self.exec_inner(false, cb)
    }

    /// Execute logic on the state maintained by this actor closing the actor.
    #[inline]
    pub fn exec_close<R, Fut, Cb>(
        &self,
        cb: Cb,
    ) -> impl std::future::Future<Output = Result<R>> + 'static + Send
    where
        R: 'static + Send,
        Fut: std::future::Future<Output = (Option<T>, Result<R>)>
            + 'static
            + Send,
        Cb: FnOnce(T) -> Fut + 'static + Send,
    {
        self.exec_inner(true, cb)
    }

    fn exec_inner<R, Fut, Cb>(
        &self,
        should_close: bool,
        cb: Cb,
    ) -> impl std::future::Future<Output = Result<R>> + 'static + Send
    where
        R: 'static + Send,
        Fut: std::future::Future<Output = (Option<T>, Result<R>)>
            + 'static
            + Send,
        Cb: FnOnce(T) -> Fut + 'static + Send,
    {
        let sender = if should_close {
            if let Some(sender) = self.0.lock().take() {
                Ok(sender)
            } else {
                Err(Error::id("Closed"))
            }
        } else {
            match &*self.0.lock() {
                None => Err(Error::id("Closed")),
                Some(sender) => Ok(sender.clone()),
            }
        };
        async move {
            let sender = sender?;
            let (s, r) = tokio::sync::oneshot::channel();
            if sender
                .send(Box::new(|t| {
                    Box::pin(async move {
                        let (t, r) = cb(t).await;
                        let _ = s.send(r);
                        t
                    })
                }))
                .is_err()
            {
                return Err(Error::id("Closed"));
            }
            r.await.map_err(|_| Error::id("Closed"))?
        }
    }
}
