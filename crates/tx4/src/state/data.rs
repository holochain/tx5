#![allow(dead_code)]

use super::*;

use std::collections::{hash_map, HashMap};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Key(pub u64);

fn uniq() -> Key {
    use std::sync::atomic::{AtomicU64, Ordering};
    static UNIQ: AtomicU64 = AtomicU64::new(0);
    Key(UNIQ.fetch_add(1, Ordering::Relaxed))
}

pub trait Logic<T: 'static + Send>: 'static + Send {
    fn exec(self: Box<Self>, t: &mut T);
}

struct StubLogic<T: 'static + Send>(std::marker::PhantomData<fn() -> T>);

impl<T: 'static + Send> StubLogic<T> {
    pub fn new() -> Self {
        Self(std::marker::PhantomData)
    }
}

impl<T: 'static + Send> Logic<T> for StubLogic<T> {
    fn exec(self: Box<Self>, _t: &mut T) {}
}

type SysChan<T> = std::ops::ControlFlow<
    (Box<dyn Logic<T>>, Option<std::io::Error>),
    Box<dyn Logic<T>>,
>;

pub struct Sys<T: 'static + Send>(
    tokio::sync::mpsc::UnboundedSender<SysChan<T>>,
);

impl<T: 'static + Send> Clone for Sys<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T: 'static + Send> Sys<T> {
    pub fn new(mut t: T) -> Self {
        let (s, mut r) = tokio::sync::mpsc::unbounded_channel::<SysChan<T>>();
        tokio::task::spawn(async move {
            while let Some(l) = r.recv().await {
                match l {
                    std::ops::ControlFlow::Continue(l) => {
                        l.exec(&mut t);
                    }
                    std::ops::ControlFlow::Break((l, _maybe_err)) => {
                        l.exec(&mut t);
                        break;
                    }
                }
            }
        });
        Self(s)
    }

    #[inline]
    pub fn run<L: Logic<T>>(&self, logic: L) {
        let _ = self
            .0
            .send(std::ops::ControlFlow::Continue(Box::new(logic)));
    }

    pub fn run_closure<Cb>(&self, cb: Cb)
    where
        Cb: FnOnce(&mut T) + 'static + Send,
    {
        struct ClosureLogic<T: 'static + Send>(
            Box<dyn FnOnce(&mut T) + 'static + Send>,
        );
        impl<T: 'static + Send> Logic<T> for ClosureLogic<T> {
            fn exec(self: Box<Self>, t: &mut T) {
                self.0(t);
            }
        }
        self.run(ClosureLogic(Box::new(cb)));
    }

    #[inline]
    pub fn run_final<L: Logic<T>>(
        &self,
        logic: L,
        maybe_err: Option<std::io::Error>,
    ) {
        let _ = self
            .0
            .send(std::ops::ControlFlow::Break((Box::new(logic), maybe_err)));
    }

    pub fn run_final_closure<Cb>(
        &self,
        cb: Cb,
        maybe_err: Option<std::io::Error>,
    ) where
        Cb: FnOnce(&mut T) + 'static + Send,
    {
        struct ClosureLogic<T: 'static + Send>(
            Box<dyn FnOnce(&mut T) + 'static + Send>,
        );
        impl<T: 'static + Send> Logic<T> for ClosureLogic<T> {
            fn exec(self: Box<Self>, t: &mut T) {
                self.0(t);
            }
        }
        self.run_final(ClosureLogic(Box::new(cb)), maybe_err);
    }

    #[inline]
    pub fn close(&self, maybe_err: Option<std::io::Error>) {
        self.run_final(StubLogic::new(), maybe_err);
    }
}

struct SigStateDataInner {
    key: Key,
    init_ok: bool,
    init_cb_list: Vec<tokio::sync::oneshot::Sender<Result<()>>>,
    sig_evt: tokio::sync::mpsc::UnboundedSender<Result<SigStateEvt>>,
}

#[derive(Clone)]
pub struct SigStateData(Sys<SigStateDataInner>);

impl SigStateData {
    pub fn new(
        maybe_respond: Option<tokio::sync::oneshot::Sender<Result<()>>>,
    ) -> (Self, Key, ManyRcv<Result<SigStateEvt>>) {
        let (sig_evt_send, sig_evt_recv) =
            tokio::sync::mpsc::unbounded_channel();

        let key = uniq();

        let init_cb_list = match maybe_respond {
            Some(r) => vec![r],
            None => Vec::new(),
        };

        let inner = SigStateDataInner {
            key,
            init_ok: false,
            init_cb_list,
            sig_evt: sig_evt_send,
        };

        (Self(Sys::new(inner)), key, ManyRcv(sig_evt_recv))
    }

    pub fn shutdown(&self, err: std::io::Error) {
        let err2 = err.err_clone();
        self.0.run_final_closure(
            move |inner| {
                for resp in inner.init_cb_list.drain(..) {
                    let _ = resp.send(Err(err2.err_clone()));
                }
                let _ = inner.sig_evt.send(Err(err2));
            },
            Some(err),
        );
    }

    pub fn push_assert_respond(
        &self,
        respond: tokio::sync::oneshot::Sender<Result<()>>,
    ) {
        self.0.run_closure(move |inner| {
            if inner.init_ok {
                let _ = respond.send(Ok(()));
            } else {
                inner.init_cb_list.push(respond);
            }
        });
    }

    pub fn init_ok(&self) {
        self.0.run_closure(move |inner| {
            inner.init_ok = true;
            for resp in inner.init_cb_list.drain(..) {
                let _ = resp.send(Ok(()));
            }
        });
    }

    pub fn send_offer(
        &self,
        state: StateData,
        sig_key: Key,
        conn_key: Key,
        sig_url: Tx4Url,
        rem_id: Id,
        offer: Buf,
    ) {
        self.0.run_closure(move |inner| {
            let snd = OneSnd::new(move |res| {
                if let Err(err) = res {
                    state.new_listener_sig_err(
                        sig_key,
                        sig_url,
                        err.err_clone(),
                    );
                    state.new_conn_err(conn_key, rem_id, err);
                }
            });

            let _ = inner
                .sig_evt
                .send(Ok(SigStateEvt::SndOffer(rem_id, offer, snd)));
        });
    }
}

struct ConnStateDataInner {
    key: Key,
    connected: bool,
    send_list: Vec<(Buf, tokio::sync::oneshot::Sender<Result<()>>)>,
    conn_evt: tokio::sync::mpsc::UnboundedSender<Result<ConnStateEvt>>,
}

pub struct ConnStateData(Sys<ConnStateDataInner>);

impl ConnStateData {
    pub fn new(
        maybe_send: Option<(Buf, tokio::sync::oneshot::Sender<Result<()>>)>,
    ) -> (Self, Key, ManyRcv<Result<ConnStateEvt>>) {
        let (conn_evt_send, conn_evt_recv) =
            tokio::sync::mpsc::unbounded_channel();

        let key = uniq();

        let send_list = match maybe_send {
            Some(s) => vec![s],
            None => Vec::new(),
        };

        let inner = ConnStateDataInner {
            key,
            connected: false,
            send_list,
            conn_evt: conn_evt_send,
        };

        (Self(Sys::new(inner)), key, ManyRcv(conn_evt_recv))
    }

    pub fn shutdown(&self, err: std::io::Error) {
        let err2 = err.err_clone();
        self.0.run_final_closure(
            move |inner| {
                for (_, resp) in inner.send_list.drain(..) {
                    let _ = resp.send(Err(err2.err_clone()));
                }
                let _ = inner.conn_evt.send(Err(err2));
            },
            Some(err),
        );
    }

    pub fn create_offer(&self, snd: OneSnd<Buf>) {
        self.0.run_closure(move |inner| {
            let _ = inner.conn_evt.send(Ok(ConnStateEvt::CreateOffer(snd)));
        });
    }

    pub fn set_rem(&self, answer: Buf) {
        self.0.run_closure(move |inner| {
            let snd = OneSnd::new(move |_res| todo!());
            let _ = inner.conn_evt.send(Ok(ConnStateEvt::SetRem(answer, snd)));
        });
    }
}

struct StateDataInner {
    evt: tokio::sync::mpsc::UnboundedSender<Result<StateEvt>>,
    signal_map: HashMap<Tx4Url, (Key, SigStateData)>,
    conn_map: HashMap<Id, (Key, ConnStateData)>,
}

#[derive(Clone)]
pub struct StateData(Sys<StateDataInner>);

impl StateData {
    pub fn new(
        evt: tokio::sync::mpsc::UnboundedSender<Result<StateEvt>>,
    ) -> Self {
        Self(Sys::new(StateDataInner {
            evt,
            signal_map: HashMap::new(),
            conn_map: HashMap::new(),
        }))
    }

    pub fn shutdown(&self, err: std::io::Error) {
        let err2 = err.err_clone();
        self.0.run_final_closure(
            move |inner| {
                for (_, (_, sig)) in inner.signal_map.drain() {
                    sig.shutdown(err2.err_clone());
                }
                for (_, (_, conn)) in inner.conn_map.drain() {
                    conn.shutdown(err2.err_clone());
                }
                let _ = inner.evt.send(Err(err2));
            },
            Some(err),
        );
    }

    pub async fn assert_listener_sig(&self, sig_url: Tx4Url) -> Result<()> {
        let (s, r) = tokio::sync::oneshot::channel();
        self.0.run(AssertListenerSig {
            state: self.clone(),
            sig_url,
            maybe_respond: Some(s),
        });
        r.await.map_err(|_| Error::id("Closed"))?
    }

    pub fn new_listener_sig_ok(
        &self,
        key: Key,
        sig_url: Tx4Url,
        cli_url: Tx4Url,
    ) {
        self.0.run_closure(move |inner| {
            if let Some((sig_key, sig)) = inner.signal_map.get(&sig_url) {
                if *sig_key != key {
                    // whoops! that wasn't us!
                    return;
                }
                sig.init_ok();
                let _ = inner.evt.send(Ok(StateEvt::Address(cli_url)));
            }
        });
    }

    pub fn new_listener_sig_err(
        &self,
        key: Key,
        sig_url: Tx4Url,
        err: std::io::Error,
    ) {
        self.0.run_closure(move |inner| {
            if let Some((sig_key, sig)) = inner.signal_map.remove(&sig_url) {
                if sig_key != key {
                    // whoops! that wasn't us!
                    inner.signal_map.insert(sig_url, (sig_key, sig));
                    return;
                }
                sig.shutdown(err);
            }
        });
    }

    pub async fn assert_conn_for_send(
        &self,
        cli_url: Tx4Url,
        data: Buf,
    ) -> Result<()> {
        let (s, r) = tokio::sync::oneshot::channel();

        self.0.run(AssertConn {
            state: self.clone(),
            sig_url: cli_url.to_server(),
            rem_id: cli_url.id().unwrap(),
            maybe_send: Some((data, s)),
        });

        r.await.map_err(|_| Error::id("Closed"))?
    }

    pub fn new_conn_ok(&self, key: Key, rem_id: Id, sig_url: Tx4Url) {
        let state = self.clone();
        self.0.run_closure(move |inner| {
            if let Some((conn_key, conn)) = inner.conn_map.get_mut(&rem_id) {
                if *conn_key != key {
                    // whoops! that wasn't us!
                    return;
                }
                let rem_id = rem_id.clone();
                let sig_url = sig_url.clone();
                let snd = OneSnd::new(move |offer| match offer {
                    Ok(offer) => {
                        state.new_conn_send_offer(key, rem_id, sig_url, offer)
                    }
                    Err(err) => state.new_conn_err(key, rem_id, err),
                });
                conn.create_offer(snd);
            }
        });
    }

    pub fn new_conn_err(&self, key: Key, rem_id: Id, err: std::io::Error) {
        self.0.run_closure(move |inner| {
            if let Some((conn_key, conn)) = inner.conn_map.remove(&rem_id) {
                if conn_key != key {
                    // whoops! that wasn't us!
                    inner.conn_map.insert(rem_id, (conn_key, conn));
                    return;
                }
                conn.shutdown(err);
            }
        });
    }

    pub fn new_conn_send_offer(
        &self,
        conn_key: Key,
        rem_id: Id,
        sig_url: Tx4Url,
        offer: Buf,
    ) {
        let state = self.clone();
        self.0.run_closure(move |inner| {
            AssertListenerSig::call(
                inner,
                state.clone(),
                sig_url.clone(),
                None,
            );
            let (sig_key, sig) = inner.signal_map.get(&sig_url).unwrap();
            sig.send_offer(state, *sig_key, conn_key, sig_url, rem_id, offer);
        });
    }

    pub fn check_incoming_answer(
        &self,
        _sig_url: Tx4Url,
        rem_id: Id,
        answer: Buf,
    ) {
        self.0.run_closure(move |inner| {
            if let Some((_conn_key, conn)) = inner.conn_map.get_mut(&rem_id) {
                conn.set_rem(answer)
            }
        });
    }
}

struct AssertListenerSig {
    state: StateData,
    sig_url: Tx4Url,
    maybe_respond: Option<tokio::sync::oneshot::Sender<Result<()>>>,
}

impl AssertListenerSig {
    pub fn call(
        inner: &mut StateDataInner,
        state: StateData,
        sig_url: Tx4Url,
        maybe_respond: Option<tokio::sync::oneshot::Sender<Result<()>>>,
    ) {
        Box::new(AssertListenerSig {
            state,
            sig_url,
            maybe_respond,
        })
        .exec(inner);
    }
}

impl Logic<StateDataInner> for AssertListenerSig {
    fn exec(self: Box<Self>, inner: &mut StateDataInner) {
        let AssertListenerSig {
            state,
            sig_url,
            maybe_respond,
        } = *self;
        match inner.signal_map.entry(sig_url.clone()) {
            hash_map::Entry::Occupied(e) => {
                if let Some(respond) = maybe_respond {
                    e.get().1.push_assert_respond(respond);
                }
            }
            hash_map::Entry::Vacant(e) => {
                let (sig_state_data, key, sig_evt) =
                    SigStateData::new(maybe_respond);
                e.insert((key, sig_state_data));
                let seed = SigStateSeed {
                    done: false,
                    key,
                    sig_url: sig_url.clone(),
                    state_data: state,
                    sig_evt: Some(sig_evt),
                };
                let _ = inner.evt.send(Ok(StateEvt::NewSig(seed)));
            }
        }
    }
}

struct AssertConn {
    state: StateData,
    sig_url: Tx4Url,
    rem_id: Id,
    maybe_send: Option<(Buf, tokio::sync::oneshot::Sender<Result<()>>)>,
}

impl Logic<StateDataInner> for AssertConn {
    fn exec(self: Box<Self>, inner: &mut StateDataInner) {
        let AssertConn {
            state,
            sig_url,
            rem_id,
            maybe_send,
        } = *self;
        if let hash_map::Entry::Vacant(e) = inner.conn_map.entry(rem_id) {
            let (conn_state_data, key, conn_evt) =
                ConnStateData::new(maybe_send);
            e.insert((key, conn_state_data));
            let seed = ConnStateSeed {
                done: false,
                key,
                rem_id,
                sig_url: sig_url.clone(),
                state_data: state,
                conn_evt: Some(conn_evt),
            };
            let _ = inner.evt.send(Ok(StateEvt::NewConn(seed)));
        }
    }
}
