#![allow(dead_code)]

use super::*;

use parking_lot::Mutex;
use std::sync::{Arc, Weak};

/// Guard allowing mut access to a store's inner type.
pub struct AccessMut<'lt, T: 'static + Send> {
    guard: parking_lot::MutexGuard<'lt, Option<T>>,
    close: bool,
    #[allow(clippy::type_complexity)] // c'mon clippy, this isn't that bad
    defer: Vec<Box<dyn FnOnce(&StoreWeak<T>) + 'static + Send>>,
}

impl<'lt, T: 'static + Send> AccessMut<'lt, T> {
    /// Schedule for close on guard drop.
    #[inline]
    pub fn close(&mut self) {
        self.close = true;
    }

    /// Schedule to run on guard drop (after mutex is unlocked).
    #[inline]
    pub fn defer<Cb>(&mut self, cb: Cb)
    where
        Cb: FnOnce(&StoreWeak<T>) + 'static + Send,
    {
        self.defer.push(Box::new(cb))
    }
}

impl<'lt, T: 'static + Send> std::ops::Deref for AccessMut<'lt, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.guard.deref().as_ref().unwrap()
    }
}

impl<'lt, T: 'static + Send> std::ops::DerefMut for AccessMut<'lt, T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.guard.deref_mut().as_mut().unwrap()
    }
}

/// Weak reference to store.
pub struct StoreWeak<T: 'static + Send>(Weak<Mutex<Option<T>>>);

impl<T: 'static + Send> Clone for StoreWeak<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T: 'static + Send> StoreWeak<T> {
    /// Upgrade to a strong reference, if there is at least 1 other strong ref.
    pub fn upgrade(&self) -> Option<Store<T>> {
        match self.0.upgrade() {
            Some(s) => {
                let out = Store(s);
                if out.is_closed() {
                    None
                } else {
                    Some(out)
                }
            }
            None => None,
        }
    }

    /// Same as Store::access_mut, but processes through the weak reference.
    pub fn access_mut<Cb, R>(&self, cb: Cb) -> Result<R>
    where
        Cb: FnOnce(&mut AccessMut<'_, T>) -> Result<R>,
    {
        match self.0.upgrade() {
            None => Err(Error::id("Closed")),
            Some(s) => Store(s).access_mut(cb),
        }
    }
}

/// Store synchronized data.
pub struct Store<T: 'static + Send>(Arc<Mutex<Option<T>>>);

impl<T: 'static + Send> Clone for Store<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T: 'static + Send> Store<T> {
    /// Create a new synchronized data store.
    #[inline]
    pub fn new(t: T) -> Self {
        Self(Arc::new(Mutex::new(Some(t))))
    }

    /// Get a weak ref to this store that will not prevent drop.
    #[inline]
    pub fn weak(&self) -> StoreWeak<T> {
        StoreWeak(Arc::downgrade(&self.0))
    }

    /// Check if this store has been closed.
    #[inline]
    pub fn is_closed(&self) -> bool {
        self.0.lock().is_none()
    }

    /// Close the store, droping the inner type.
    #[inline]
    pub fn close(&self) {
        self.0.lock().take();
    }

    /// This is the main store access function.
    /// The callback receives an AccessMut guard allowing
    /// access to the inner type, as well as utilities,
    /// such as closing the store and defering logic until
    /// after the lock is released.
    pub fn access_mut<Cb, R>(&self, cb: Cb) -> Result<R>
    where
        Cb: FnOnce(&mut AccessMut<'_, T>) -> Result<R>,
    {
        // lock our mutex
        let lock = self.0.lock();

        // check if we're closed
        if lock.is_none() {
            return Err(Error::id("Closed"));
        }

        // build our accessor
        let mut g = AccessMut {
            guard: lock,
            close: false,
            defer: Vec::new(),
        };

        // exec the access cb
        let r = cb(&mut g);

        let AccessMut {
            mut guard,
            close,
            defer,
        } = g;

        // close if instructed to do so
        if close {
            *guard = None;
        }

        // release the lock
        drop(guard);

        // run our deferred logic
        let weak = self.weak();
        for d in defer {
            d(&weak);
        }

        // return the result
        r
    }
}
