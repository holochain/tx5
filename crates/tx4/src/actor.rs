//! Quick-n-dirty actor system for the tx4 state.

use crate::*;
use parking_lot::Mutex;
use std::sync::{Arc, Weak};

type ManySnd<T> = tokio::sync::mpsc::UnboundedSender<Result<T>>;

/// Generic receiver type.
pub struct ManyRcv<T: 'static + Send>(
    pub(crate) tokio::sync::mpsc::UnboundedReceiver<Result<T>>,
);

impl<T: 'static + Send> ManyRcv<T> {
    /// Receive data from this receiver type.
    #[inline]
    pub async fn recv(&mut self) -> Option<Result<T>> {
        tokio::sync::mpsc::UnboundedReceiver::recv(&mut self.0).await
    }
}

/// Weak actor handle that does not add to reference count.
pub struct ActorWeak<T: 'static + Send>(Weak<Mutex<Option<ManySnd<T>>>>);

impl<T: 'static + Send> ActorWeak<T> {
    /// Attempt to upgrade to a full actor handle.
    pub fn upgrade(&self) -> Option<Actor<T>> {
        match self.0.upgrade() {
            None => None,
            Some(a) => {
                if a.lock().is_some() {
                    Some(Actor(a))
                } else {
                    None
                }
            }
        }
    }
}

impl<T: 'static + Send> PartialEq for ActorWeak<T> {
    fn eq(&self, rhs: &Self) -> bool {
        Weak::ptr_eq(&self.0, &rhs.0)
    }
}

impl<T: 'static + Send> Eq for ActorWeak<T> {}

impl<T: 'static + Send> PartialEq<Actor<T>> for ActorWeak<T> {
    fn eq(&self, rhs: &Actor<T>) -> bool {
        Weak::ptr_eq(&self.0, &Arc::downgrade(&rhs.0))
    }
}

impl<T: 'static + Send> Clone for ActorWeak<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

/// An actor that executes as a task, providing synchronized messaging.
pub struct Actor<T: 'static + Send>(Arc<Mutex<Option<ManySnd<T>>>>);

impl<T: 'static + Send> PartialEq for Actor<T> {
    fn eq(&self, rhs: &Self) -> bool {
        Arc::ptr_eq(&self.0, &rhs.0)
    }
}

impl<T: 'static + Send> Eq for Actor<T> {}

impl<T: 'static + Send> PartialEq<ActorWeak<T>> for Actor<T> {
    fn eq(&self, rhs: &ActorWeak<T>) -> bool {
        Weak::ptr_eq(&Arc::downgrade(&self.0), &rhs.0)
    }
}

impl<T: 'static + Send> Clone for Actor<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T: 'static + Send> Actor<T> {
    /// Construct a new actor.
    pub fn new<Fut, Cb>(cb: Cb) -> Self
    where
        Fut: std::future::Future<Output = Result<()>> + 'static + Send,
        Cb: FnOnce(ActorWeak<T>, ManyRcv<T>) -> Fut + 'static + Send,
    {
        let (s, r) = tokio::sync::mpsc::unbounded_channel();
        let out = Self(Arc::new(Mutex::new(Some(s))));
        let weak = out.weak();
        tokio::task::spawn(cb(weak, ManyRcv(r)));
        out
    }

    /// Get a weak handle to the actor that does not add to reference count.
    pub fn weak(&self) -> ActorWeak<T> {
        ActorWeak(Arc::downgrade(&self.0))
    }

    /// Check if this handle is pointing to a closed actor.
    pub fn is_closed(&self) -> bool {
        match &*self.0.lock() {
            None => true,
            Some(s) => s.is_closed(),
        }
    }

    /// Close this actor, stopping the task with an error if it is running.
    pub fn close(&self, err: std::io::Error) {
        let mut l = self.0.lock();
        if let Some(s) = &*l {
            let _ = s.send(Err(err));
        }
        let _ = l.take();
    }

    /// Send a message to the actor task.
    /// If the message sent is an Err variant, the task will be closed.
    pub fn send(&self, t: Result<T>) -> Result<()> {
        let mut res = Err(Error::id("Closed"));
        let close = t.is_err();
        let mut l = self.0.lock();
        if let Some(s) = &*l {
            if s.send(t).is_ok() {
                res = Ok(());
            }
        }
        if close {
            let _ = l.take();
        }
        res
    }
}
