//! Utilities.

use crate::*;
use std::future::Future;
use std::sync::atomic;
use std::sync::Arc;

/// Terminal notification helper utility.
#[derive(Clone)]
pub struct Term {
    term_note: &'static str,
    term: Arc<atomic::AtomicBool>,
    sig: Arc<event_listener::Event>,
    trgr: Arc<dyn Fn() + 'static + Send + Sync>,
}

impl Term {
    /// Construct a new term instance with optional term callback.
    pub fn new(
        term_note: &'static str,
        trgr: Option<Arc<dyn Fn() + 'static + Send + Sync>>,
    ) -> Self {
        let trgr = trgr.unwrap_or_else(|| Arc::new(|| {}));
        Self {
            term_note,
            term: Arc::new(atomic::AtomicBool::new(false)),
            sig: Arc::new(event_listener::Event::new()),
            trgr,
        }
    }

    /// Trigger termination.
    pub fn term(&self) {
        self.term.store(true, atomic::Ordering::Release);
        self.sig.notify(usize::MAX);
        (self.trgr)();
    }

    /// Returns `true` if termination has been triggered.
    pub fn is_term(&self) -> bool {
        self.term.load(atomic::Ordering::Acquire)
    }

    /// Returns a future that will resolve when termination is triggered.
    pub fn on_term(&self) -> impl std::future::Future<Output = ()> + 'static + Send {
        let l = self.sig.listen();
        let term = self.term.clone();
        async move {
            if term.load(atomic::Ordering::Acquire) {
                return;
            }
            l.await;
        }
    }

    fn spawn_err_inner<F, E>(&self, f: F, e: Option<E>)
    where
        F: 'static + Send + Future<Output = Result<()>>,
        E: 'static + Send + FnOnce(std::io::Error),
    {
        let term_note = self.term_note;
        let on_term_fut = self.on_term();
        tokio::task::spawn(async move {
            if let Err(err) = tokio::select! {
                _ = on_term_fut => Err(Error::err(term_note)),
                r = f => r,
            } {
                if let Some(e) = e {
                    e(err);
                }
            }
        });
    }

    /// Spawn a task that will be terminated on_term.
    /// If terminated, the future will be dropped / cancelled.
    /// If the future errors, the error callback will be invoked.
    pub fn spawn_err<F, E>(&self, f: F, e: E)
    where
        F: 'static + Send + Future<Output = Result<()>>,
        E: 'static + Send + FnOnce(std::io::Error),
    {
        self.spawn_err_inner(f, Some(e));
    }

    /// Spawn a task that will be terminated on_term.
    /// If terminated, the future will be dropped / cancelled.
    pub fn spawn<F>(&self, f: F)
    where
        F: 'static + Send + Future<Output = Result<()>>,
    {
        self.spawn_err_inner(f, StubE::None);
    }

    fn spawn_err2_inner<F, E>(term1: &Term, term2: &Term, f: F, e: Option<E>)
    where
        F: 'static + Send + Future<Output = Result<()>>,
        E: 'static + Send + FnOnce(std::io::Error),
    {
        let term_note1 = term1.term_note;
        let term_note2 = term2.term_note;
        let term1_fut = term1.on_term();
        let term2_fut = term2.on_term();
        tokio::task::spawn(async move {
            if let Err(err) = tokio::select! {
                _ = term1_fut => Err(Error::err(term_note1)),
                _ = term2_fut => Err(Error::err(term_note2)),
                r = f => r,
            } {
                if let Some(e) = e {
                    e(err);
                }
            }
        });
    }

    /// Like spawn_err, but terminates if either of two terms terminate.
    pub fn spawn_err2<F, E>(term1: &Term, term2: &Term, f: F, e: E)
    where
        F: 'static + Send + Future<Output = Result<()>>,
        E: 'static + Send + FnOnce(std::io::Error),
    {
        Term::spawn_err2_inner(term1, term2, f, Some(e));
    }

    /// Like spawn, but terminates if either of two terms terminate.
    pub fn spawn2<F>(term1: &Term, term2: &Term, f: F)
    where
        F: 'static + Send + Future<Output = Result<()>>,
    {
        Term::spawn_err2_inner(term1, term2, f, StubE::None);
    }
}

type StubE = Option<Box<dyn FnOnce(std::io::Error) + 'static + Send>>;
