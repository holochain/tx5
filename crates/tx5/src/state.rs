//! Tx5 high-level conn mgmt state.

use crate::actor::*;
use crate::*;

use std::collections::VecDeque;
use std::collections::{hash_map, HashMap};
use std::future::Future;
use std::sync::Arc;

use influxive_otel_atomic_obs::*;
use opentelemetry_api::metrics::MeterProvider;

use tx5_core::{Id, Tx5Url};

mod sig;
pub use sig::*;

mod conn;
pub use conn::*;

mod drop_consider;

#[cfg(test)]
mod test;

/// The max connection open time. Would be nice for this to be a negotiation,
/// so that it could be configured... but right now we just need both sides
/// to agree, so it is hard-coded.
const MAX_CON_TIME: std::time::Duration =
    std::time::Duration::from_secs(60 * 5);

/// The connection send grace period. Connections will not send new messages
/// when within this duration from the MAX_CON_TIME close.
/// Similar to MAX_CON_TIME, this has to be hard-coded for now.
const CON_CLOSE_SEND_GRACE: std::time::Duration =
    std::time::Duration::from_secs(30);

/// Respond type.
#[must_use]
pub struct OneSnd<T: 'static + Send>(
    Option<Box<dyn FnOnce(Result<T>) + 'static + Send>>,
);

impl<T: 'static + Send> Drop for OneSnd<T> {
    fn drop(&mut self) {
        self.send(Err(Error::id("Dropped")))
    }
}

impl<T: 'static + Send> OneSnd<T> {
    pub(crate) fn new<Cb>(cb: Cb) -> Self
    where
        Cb: FnOnce(Result<T>) + 'static + Send,
    {
        Self(Some(Box::new(cb)))
    }

    /// Send data on this single sender respond type.
    pub fn send(&mut self, t: Result<T>) {
        if let Some(sender) = self.0.take() {
            sender(t);
        }
    }

    /// Wrap such that a closure's result is sent.
    pub async fn with<Fut, Cb>(&mut self, cb: Cb)
    where
        Fut: Future<Output = Result<T>>,
        Cb: FnOnce() -> Fut,
    {
        self.send(cb().await);
    }
}

/// Drop this when you consider the data "received".
pub struct Permit(Vec<tokio::sync::OwnedSemaphorePermit>);

impl std::fmt::Debug for Permit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Permit").finish()
    }
}

/// State wishes to invoke an action.
pub enum StateEvt {
    /// Request to create a new signal client connection.
    NewSig(Tx5Url, SigStateSeed),

    /// Indicates the current node is addressable at the given url.
    Address(Tx5Url),

    /// Request to create a new webrtc peer connection.
    NewConn(Arc<serde_json::Value>, ConnStateSeed),

    /// Incoming data received on a peer connection.
    RcvData(Tx5Url, Box<dyn bytes::Buf + 'static + Send>, Vec<Permit>),

    /// Received a demo broadcast.
    Demo(Tx5Url),

    /// This is an informational notification indicating a connection
    /// has been successfully established. Unlike 'NewConn' above,
    /// no action is required, other than to let your users know.
    Connected(Tx5Url),

    /// This is an informational notification indicating a connection has
    /// been dropped. No action is required, other than to let your users know.
    /// Note, you may get disconnected events for connections that were never
    /// successfully established.
    Disconnected(Tx5Url),
}

impl std::fmt::Debug for StateEvt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StateEvt::NewSig(url, _seed) => f
                .debug_struct("StateEvt::NewSig")
                .field("url", url)
                .finish(),
            StateEvt::Address(url) => f
                .debug_struct("StateEvt::Address")
                .field("url", url)
                .finish(),
            StateEvt::NewConn(info, _seed) => f
                .debug_struct("StateEvt::NewConn")
                .field("info", info)
                .finish(),
            StateEvt::RcvData(url, data, _permit) => {
                let data_len = data.remaining();
                f.debug_struct("StateEvt::RcvData")
                    .field("url", url)
                    .field("data_len", &data_len)
                    .finish()
            }
            StateEvt::Demo(url) => {
                f.debug_struct("StateEvt::Demo").field("url", url).finish()
            }
            StateEvt::Connected(url) => f
                .debug_struct("StateEvt::Connected")
                .field("url", url)
                .finish(),
            StateEvt::Disconnected(url) => f
                .debug_struct("StateEvt::Disconnected")
                .field("url", url)
                .finish(),
        }
    }
}

#[derive(Clone)]
struct StateEvtSnd(tokio::sync::mpsc::UnboundedSender<Result<StateEvt>>);

impl StateEvtSnd {
    pub fn err(&self, err: std::io::Error) {
        let _ = self.0.send(Err(err));
    }

    pub fn publish(&self, evt: StateEvt) -> Result<()> {
        self.0.send(Ok(evt)).map_err(|_| Error::id("Closed"))
    }
}

pub(crate) struct SendData {
    msg_uniq: Uniq,
    data: BackBuf,
    timestamp: std::time::Instant,
    resp: Option<tokio::sync::oneshot::Sender<Result<()>>>,
    send_permit: tokio::sync::OwnedSemaphorePermit,
}

struct IceData {
    timestamp: std::time::Instant,
    ice: BackBuf,
}

struct RmConn(StateEvtSnd, Tx5Url);

impl Drop for RmConn {
    fn drop(&mut self) {
        let _ = self.0.publish(StateEvt::Disconnected(self.1.clone()));
    }
}

struct StateData {
    state_uniq: Uniq,
    this_id: Option<Id>,
    this: StateWeak,
    meta: StateMeta,
    evt: StateEvtSnd,
    signal_map: HashMap<Tx5Url, SigStateWeak>,
    ban_map: HashMap<Id, std::time::Instant>,
    conn_map: HashMap<Id, (ConnStateWeak, RmConn)>,
    send_map: HashMap<Id, VecDeque<SendData>>,
    ice_cache: HashMap<Id, VecDeque<IceData>>,
    recv_limit: Arc<tokio::sync::Semaphore>,
}

impl Drop for StateData {
    fn drop(&mut self) {
        self.shutdown(Error::id("Dropped"));
    }
}

impl StateData {
    fn shutdown(&mut self, err: std::io::Error) {
        tracing::trace!(state_uniq = %self.state_uniq, this_id = ?self.this_id, "StateShutdown");
        for (_, sig) in self.signal_map.drain() {
            if let Some(sig) = sig.upgrade() {
                sig.close(err.err_clone());
            }
        }
        for (_, (conn, _)) in self.conn_map.drain() {
            if let Some(conn) = conn.upgrade() {
                conn.close(err.err_clone());
            }
        }
        self.send_map.clear();
        self.evt.err(err);
    }

    async fn exec(&mut self, cmd: StateCmd) -> Result<()> {
        // TODO - any errors returned by these fn calls will shut down
        //        the entire endpoint... probably not what we want.
        //        Instead, maybe shutting down the whole endpoint should
        //        require a special call, and otherwise errors can be
        //        logged / ignored OR just not allow returning errors.
        match cmd {
            StateCmd::Tick1s => self.tick_1s().await,
            StateCmd::TrackSig { rem_id, ty, bytes } => {
                self.track_sig(rem_id, ty, bytes).await
            }
            StateCmd::SndDemo => self.snd_demo().await,
            StateCmd::ListConnected(resp) => self.list_connected(resp).await,
            StateCmd::AssertListenerSig { sig_url, resp } => {
                self.assert_listener_sig(sig_url, resp).await
            }
            StateCmd::SendData {
                msg_uniq,
                rem_id,
                data,
                timestamp,
                send_permit,
                resp,
                cli_url,
            } => {
                self.send_data(
                    msg_uniq,
                    rem_id,
                    data,
                    timestamp,
                    send_permit,
                    resp,
                    cli_url,
                )
                .await
            }
            StateCmd::Ban { rem_id, span } => self.ban(rem_id, span).await,
            StateCmd::Stats(resp) => self.stats(resp).await,
            StateCmd::Publish { evt } => self.publish(evt).await,
            StateCmd::SigConnected { cli_url } => {
                self.sig_connected(cli_url).await
            }
            StateCmd::FetchForSend { conn, rem_id } => {
                self.fetch_for_send(conn, rem_id).await
            }
            StateCmd::InOffer {
                sig_url,
                rem_id,
                data,
            } => self.in_offer(sig_url, rem_id, data).await,
            StateCmd::InDemo { sig_url, rem_id } => {
                self.in_demo(sig_url, rem_id).await
            }
            StateCmd::CacheIce { rem_id, ice } => {
                self.cache_ice(rem_id, ice).await
            }
            StateCmd::GetCachedIce { rem_id } => {
                self.get_cached_ice(rem_id).await
            }
            StateCmd::CloseSig { sig_url, sig, err } => {
                self.close_sig(sig_url, sig, err).await
            }
            StateCmd::ConnReady { cli_url } => self.conn_ready(cli_url).await,
            StateCmd::CloseConn { rem_id, conn, err } => {
                self.close_conn(rem_id, conn, err).await
            }
        }
    }

    async fn tick_1s(&mut self) -> Result<()> {
        let timeout = self.meta.config.max_conn_init();

        self.ice_cache.retain(|_, list| {
            list.retain_mut(|data| data.timestamp.elapsed() < timeout);
            !list.is_empty()
        });

        self.send_map.retain(|_, list| {
            list.retain_mut(|info| {
                if info.timestamp.elapsed() < timeout {
                    true
                } else {
                    tracing::trace!(msg_uniq = %info.msg_uniq, "dropping msg due to timeout");
                    if let Some(resp) = info.resp.take() {
                        let _ = resp.send(Err(Error::id("Timeout")));
                    }
                    false
                }
            });
            !list.is_empty()
        });

        let tot_conn_cnt = self.meta.metric_conn_count.get();

        let mut tot_snd_bytes = 0;
        let mut tot_rcv_bytes = 0;
        let mut tot_age = 0.0;
        let mut age_cnt = 0.0;

        for (_, (conn, _)) in self.conn_map.iter() {
            let meta = conn.meta();
            tot_snd_bytes += meta.metric_bytes_snd.get();
            tot_rcv_bytes += meta.metric_bytes_rcv.get();
            tot_age += meta.created_at.elapsed().as_secs_f64();
            age_cnt += 1.0;
        }

        let tot_avg_age_s = if age_cnt > 0.0 {
            tot_age / age_cnt
        } else {
            0.0
        };

        self.conn_map.retain(|_, (conn, _)| {
            let meta = conn.meta();

            let args = drop_consider::DropConsiderArgs {
                conn_uniq: meta.conn_uniq.clone(),
                cfg_conn_max_cnt: meta.config.max_conn_count() as i64,
                cfg_conn_max_init: meta.config.max_conn_init().as_secs_f64(),
                tot_conn_cnt,
                tot_snd_bytes,
                tot_rcv_bytes,
                tot_avg_age_s,
                this_connected: meta
                    .connected
                    .load(std::sync::atomic::Ordering::SeqCst),
                this_snd_bytes: meta.metric_bytes_snd.get(),
                this_rcv_bytes: meta.metric_bytes_rcv.get(),
                this_age_s: meta.created_at.elapsed().as_secs_f64(),
                this_last_active_s: meta.last_active_at.elapsed().as_secs_f64(),
            };

            if let drop_consider::DropConsiderResult::MustDrop =
                drop_consider::drop_consider(&args)
            {
                if let Some(conn) = conn.upgrade() {
                    conn.close(Error::id("DropContention"));
                }
                return false;
            }

            true
        });

        Ok(())
    }

    async fn track_sig(
        &mut self,
        rem_id: Id,
        ty: &'static str,
        bytes: usize,
    ) -> Result<()> {
        if let Some((conn, _)) = self.conn_map.get(&rem_id) {
            if let Some(conn) = conn.upgrade() {
                conn.track_sig(ty, bytes);
            }
        }
        Ok(())
    }

    async fn snd_demo(&mut self) -> Result<()> {
        for (_, sig) in self.signal_map.iter() {
            if let Some(sig) = sig.upgrade() {
                sig.snd_demo();
            }
        }

        Ok(())
    }

    async fn list_connected(
        &mut self,
        resp: tokio::sync::oneshot::Sender<Result<Vec<Tx5Url>>>,
    ) -> Result<()> {
        let mut urls = Vec::new();
        for (_, (con, _)) in self.conn_map.iter() {
            if let Some(con) = con.upgrade() {
                urls.push(con.rem_url());
            }
        }
        resp.send(Ok(urls)).map_err(|_| Error::id("Closed"))
    }

    async fn assert_listener_sig(
        &mut self,
        sig_url: Tx5Url,
        resp: tokio::sync::oneshot::Sender<Result<Tx5Url>>,
    ) -> Result<()> {
        tracing::debug!(state_uniq = %self.state_uniq, %sig_url, "begin register with signal server");
        let new_sig = |resp| -> SigState {
            let (sig, sig_evt) = SigState::new(
                self.this.clone(),
                sig_url.clone(),
                resp,
                self.meta.config.max_conn_init(),
            );
            let seed = SigStateSeed::new(sig.clone(), sig_evt);
            let _ = self.evt.publish(StateEvt::NewSig(sig_url.clone(), seed));
            sig
        };
        match self.signal_map.entry(sig_url.clone()) {
            hash_map::Entry::Occupied(mut e) => match e.get().upgrade() {
                Some(sig) => sig.push_assert_respond(resp).await,
                None => {
                    let sig = new_sig(resp);
                    e.insert(sig.weak());
                }
            },
            hash_map::Entry::Vacant(e) => {
                let sig = new_sig(resp);
                e.insert(sig.weak());
            }
        }
        Ok(())
    }

    fn is_banned(&mut self, rem_id: Id) -> bool {
        let now = std::time::Instant::now();
        self.ban_map.retain(|_id, expires_at| *expires_at > now);
        self.ban_map.contains_key(&rem_id)
    }

    async fn create_new_conn(
        &mut self,
        sig_url: Tx5Url,
        rem_id: Id,
        maybe_offer: Option<BackBuf>,
        maybe_msg_uniq: Option<Uniq>,
    ) -> Result<()> {
        if self.is_banned(rem_id) {
            tracing::warn!(
                ?rem_id,
                "Ignoring request to create con to banned remote"
            );
            return Ok(());
            //return Err(Error::id("Ban"));
        }

        let (s, r) = tokio::sync::oneshot::channel();
        if let Err(err) = self.assert_listener_sig(sig_url.clone(), s).await {
            tracing::warn!(?err, "failed to assert signal listener");
            return Ok(());
        }

        let sig = self.signal_map.get(&sig_url).unwrap().clone();

        let conn_uniq = self.state_uniq.sub();

        tracing::trace!(?maybe_msg_uniq, %conn_uniq, "create_new_conn");

        let cli_url = sig_url.to_client(rem_id);
        let conn = match ConnState::new_and_publish(
            self.meta.config.clone(),
            self.meta.conn_limit.clone(),
            self.meta.metric_conn_count.clone(),
            self.this.clone(),
            sig,
            self.state_uniq.clone(),
            conn_uniq,
            self.this_id.unwrap(),
            cli_url.clone(),
            rem_id,
            self.recv_limit.clone(),
            r,
            maybe_offer,
            self.meta.snd_ident.clone(),
        ) {
            Err(err) => {
                tracing::warn!(?err, "failed to create conn state");
                return Ok(());
            }
            Ok(conn) => conn,
        };

        self.conn_map
            .insert(rem_id, (conn, RmConn(self.evt.clone(), cli_url)));

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn send_data(
        &mut self,
        msg_uniq: Uniq,
        rem_id: Id,
        data: BackBuf,
        timestamp: std::time::Instant,
        send_permit: tokio::sync::OwnedSemaphorePermit,
        data_sent: tokio::sync::oneshot::Sender<Result<()>>,
        cli_url: Tx5Url,
    ) -> Result<()> {
        if self.is_banned(rem_id) {
            tracing::warn!(
                ?rem_id,
                "Ignoring request to send data to banned remote"
            );
            let _ = data_sent.send(Err(Error::id("Ban")));
            return Ok(());
        }

        self.send_map
            .entry(rem_id)
            .or_default()
            .push_back(SendData {
                msg_uniq: msg_uniq.clone(),
                data,
                timestamp,
                resp: Some(data_sent),
                send_permit,
            });

        let rem_id = cli_url.id().unwrap();

        if let Some((e, _)) = self.conn_map.get(&rem_id) {
            if let Some(conn) = e.upgrade() {
                conn.check_send_waiting(None).await;
                return Ok(());
            } else {
                self.conn_map.remove(&rem_id);
            }
        }

        let sig_url = cli_url.to_server();
        self.create_new_conn(sig_url, rem_id, None, Some(msg_uniq))
            .await
    }

    async fn ban(
        &mut self,
        rem_id: Id,
        span: std::time::Duration,
    ) -> Result<()> {
        let expires_at = std::time::Instant::now() + span;
        self.ban_map.insert(rem_id, expires_at);
        self.send_map.remove(&rem_id);
        self.ice_cache.remove(&rem_id);
        if let Some((conn, _)) = self.conn_map.remove(&rem_id) {
            if let Some(conn) = conn.upgrade() {
                conn.close(Error::id("Ban"));
            }
        }
        Ok(())
    }

    fn stats(
        &mut self,
        resp: tokio::sync::oneshot::Sender<Result<serde_json::Value>>,
    ) -> impl std::future::Future<Output = Result<()>> + 'static + Send {
        let this_id =
            self.this_id.map(|id| id.to_string()).unwrap_or("".into());
        let conn_list = self
            .conn_map
            .iter()
            .map(|(id, (c, _))| (*id, c.clone()))
            .collect::<Vec<_>>();
        let now = std::time::Instant::now();
        let mut ban_map = serde_json::Map::new();
        for (id, until) in self.ban_map.iter() {
            ban_map.insert(id.to_string(), (*until - now).as_secs_f64().into());
        }
        async move {
            let mut map = serde_json::Map::new();

            #[cfg(feature = "backend-go-pion")]
            const BACKEND: &str = "go-pion";
            #[cfg(feature = "backend-webrtc-rs")]
            const BACKEND: &str = "webrtc-rs";

            map.insert("backend".into(), BACKEND.into());
            map.insert("thisId".into(), this_id.into());
            map.insert("banned".into(), ban_map.into());

            for (id, conn) in conn_list {
                if let Some(conn) = conn.upgrade() {
                    if let Ok(stats) = conn.stats().await {
                        map.insert(id.to_string(), stats);
                    }
                }
            }

            let _ = resp.send(Ok(map.into()));

            Ok(())
        }
    }

    async fn publish(&mut self, evt: StateEvt) -> Result<()> {
        let _ = self.evt.publish(evt);
        Ok(())
    }

    async fn sig_connected(&mut self, cli_url: Tx5Url) -> Result<()> {
        let loc_id = cli_url.id().unwrap();
        if let Some(this_id) = &self.this_id {
            if this_id != &loc_id {
                return Err(Error::err("MISMATCH LOCAL ID, please use the same lair instance for every sig connection"));
            }
        } else {
            self.this_id = Some(loc_id);
        }
        let _ = self.evt.publish(StateEvt::Address(cli_url));
        Ok(())
    }

    async fn fetch_for_send(
        &mut self,
        want_conn: ConnStateWeak,
        rem_id: Id,
    ) -> Result<()> {
        let conn = match self.conn_map.get(&rem_id) {
            None => return Ok(()),
            Some((cur_conn, _)) => {
                if cur_conn != &want_conn {
                    return Ok(());
                }
                match cur_conn.upgrade() {
                    None => return Ok(()),
                    Some(conn) => conn,
                }
            }
        };
        let to_send = match self.send_map.get_mut(&rem_id) {
            None => return Ok(()),
            Some(to_send) => match to_send.pop_front() {
                None => return Ok(()),
                Some(to_send) => to_send,
            },
        };
        conn.send(to_send);
        Ok(())
    }

    async fn in_offer(
        &mut self,
        sig_url: Tx5Url,
        rem_id: Id,
        offer: BackBuf,
    ) -> Result<()> {
        if let Some((e, _)) = self.conn_map.get(&rem_id) {
            if let Some(conn) = e.upgrade() {
                // we seem to have a valid conn here... but
                // we're receiving an incoming offer:
                // activate PERFECT NEGOTIATION
                // (https://developer.mozilla.org/en-US/docs/Web/API/WebRTC_API/Perfect_negotiation)

                if self.this_id.is_none() {
                    return Err(Error::err("Somehow we ended up receiving a webrtc offer before establishing a signal connection... this should be impossible"));
                }

                match self.this_id.as_ref().unwrap().cmp(&rem_id) {
                    std::cmp::Ordering::Less => {
                        //println!("OFFER_CONFLICT:BEING_POLITE");

                        // we are the POLITE node, delete our connection
                        // and set up a new one with the incoming offer.
                        self.conn_map.remove(&rem_id);
                        conn.close(Error::id(
                            "PoliteShutdownToAcceptIncomingOffer",
                        ));
                    }
                    std::cmp::Ordering::Greater => {
                        //println!("OFFER_CONFLICT:BEING_IMPOLITE");

                        // we are the IMPOLITE node, we'll ignore this
                        // offer and continue with our existing connection.
                        drop(offer);
                        return Ok(());
                    }
                    std::cmp::Ordering::Equal => {
                        tracing::warn!("Invalid incoming webrtc offer with id matching our local id. Please don't share lair connections");
                        self.conn_map.remove(&rem_id);
                        return Ok(());
                        //return Err(Error::err("Invalid incoming webrtc offer with id matching our local id. Please don't share lair connections"));
                    }
                }
            } else {
                self.conn_map.remove(&rem_id);
            }
        }

        self.create_new_conn(sig_url, rem_id, Some(offer), None)
            .await
    }

    async fn in_demo(&mut self, sig_url: Tx5Url, rem_id: Id) -> Result<()> {
        let cli_url = sig_url.to_client(rem_id);
        self.evt.publish(StateEvt::Demo(cli_url))
    }

    async fn cache_ice(&mut self, rem_id: Id, ice: BackBuf) -> Result<()> {
        let list = self.ice_cache.entry(rem_id).or_default();
        list.push_back(IceData {
            timestamp: std::time::Instant::now(),
            ice,
        });
        Ok(())
    }

    async fn get_cached_ice(&mut self, rem_id: Id) -> Result<()> {
        let StateData {
            conn_map,
            ice_cache,
            ..
        } = self;
        if let Some((conn, _)) = conn_map.get(&rem_id) {
            if let Some(conn) = conn.upgrade() {
                if let Some(list) = ice_cache.get_mut(&rem_id) {
                    for ice_data in list.iter_mut() {
                        conn.in_ice(ice_data.ice.try_clone()?, false);
                    }
                }
            }
        }
        Ok(())
    }

    async fn close_sig(
        &mut self,
        sig_url: Tx5Url,
        sig: SigStateWeak,
        err: std::io::Error,
    ) -> Result<()> {
        if let Some(cur_sig) = self.signal_map.remove(&sig_url) {
            if cur_sig == sig {
                if let Some(sig) = sig.upgrade() {
                    sig.close(err);
                }
            } else {
                // Whoops!
                self.signal_map.insert(sig_url, cur_sig);
            }
        }
        Ok(())
    }

    async fn conn_ready(&mut self, cli_url: Tx5Url) -> Result<()> {
        self.evt.publish(StateEvt::Connected(cli_url))
    }

    async fn close_conn(
        &mut self,
        rem_id: Id,
        conn: ConnStateWeak,
        err: std::io::Error,
    ) -> Result<()> {
        if let Some((cur_conn, rm)) = self.conn_map.remove(&rem_id) {
            if cur_conn == conn {
                if let Some(conn) = conn.upgrade() {
                    conn.close(err);
                }
            } else {
                // Whoops!
                self.conn_map.insert(rem_id, (cur_conn, rm));
            }
        }
        Ok(())
    }
}

enum StateCmd {
    Tick1s,
    TrackSig {
        rem_id: Id,
        ty: &'static str,
        bytes: usize,
    },
    SndDemo,
    ListConnected(tokio::sync::oneshot::Sender<Result<Vec<Tx5Url>>>),
    AssertListenerSig {
        sig_url: Tx5Url,
        resp: tokio::sync::oneshot::Sender<Result<Tx5Url>>,
    },
    SendData {
        msg_uniq: Uniq,
        rem_id: Id,
        data: BackBuf,
        timestamp: std::time::Instant,
        send_permit: tokio::sync::OwnedSemaphorePermit,
        resp: tokio::sync::oneshot::Sender<Result<()>>,
        cli_url: Tx5Url,
    },
    Ban {
        rem_id: Id,
        span: std::time::Duration,
    },
    Stats(tokio::sync::oneshot::Sender<Result<serde_json::Value>>),
    Publish {
        evt: StateEvt,
    },
    SigConnected {
        cli_url: Tx5Url,
    },
    FetchForSend {
        conn: ConnStateWeak,
        rem_id: Id,
    },
    InOffer {
        sig_url: Tx5Url,
        rem_id: Id,
        data: BackBuf,
    },
    InDemo {
        sig_url: Tx5Url,
        rem_id: Id,
    },
    CacheIce {
        rem_id: Id,
        ice: BackBuf,
    },
    GetCachedIce {
        rem_id: Id,
    },
    CloseSig {
        sig_url: Tx5Url,
        sig: SigStateWeak,
        err: std::io::Error,
    },
    ConnReady {
        cli_url: Tx5Url,
    },
    CloseConn {
        rem_id: Id,
        conn: ConnStateWeak,
        err: std::io::Error,
    },
}

#[allow(clippy::too_many_arguments)]
async fn state_task(
    mut rcv: ManyRcv<StateCmd>,
    state_uniq: Uniq,
    this: StateWeak,
    meta: StateMeta,
    evt: StateEvtSnd,
    recv_limit: Arc<tokio::sync::Semaphore>,
) -> Result<()> {
    let mut data = StateData {
        state_uniq,
        this_id: None,
        this,
        meta,
        evt,
        signal_map: HashMap::new(),
        ban_map: HashMap::new(),
        conn_map: HashMap::new(),
        send_map: HashMap::new(),
        ice_cache: HashMap::new(),
        recv_limit,
    };
    let err = match async {
        while let Some(cmd) = rcv.recv().await {
            data.exec(cmd?).await?;
        }
        Ok(())
    }
    .await
    {
        Err(err) => err,
        Ok(_) => Error::id("Dropped"),
    };
    data.shutdown(err.err_clone());
    Err(err)
}

#[derive(Clone)]
pub(crate) struct StateMeta {
    pub(crate) state_uniq: Uniq,
    pub(crate) config: DynConfig,
    pub(crate) conn_limit: Arc<tokio::sync::Semaphore>,
    pub(crate) snd_limit: Arc<tokio::sync::Semaphore>,
    pub(crate) metric_conn_count: AtomicObservableUpDownCounterI64,
    pub(crate) snd_ident: Arc<std::sync::atomic::AtomicU64>,
}

/// Weak version of State.
#[derive(Clone)]
pub struct StateWeak(ActorWeak<StateCmd>, StateMeta);

impl StateWeak {
    /// Upgrade to a full State instance.
    pub fn upgrade(&self) -> Option<State> {
        self.0.upgrade().map(|s| State(s, self.1.clone()))
    }
}

/// Handle to a state tracking instance.
#[derive(Clone)]
pub struct State(Actor<StateCmd>, StateMeta);

impl PartialEq for State {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Eq for State {}

impl State {
    /// Construct a new state instance.
    pub fn new(config: DynConfig) -> Result<(Self, ManyRcv<StateEvt>)> {
        let conn_limit = Arc::new(tokio::sync::Semaphore::new(
            config.max_conn_count() as usize,
        ));

        let snd_limit = Arc::new(tokio::sync::Semaphore::new(
            config.max_send_bytes() as usize,
        ));
        let rcv_limit = Arc::new(tokio::sync::Semaphore::new(
            config.max_recv_bytes() as usize,
        ));

        let state_uniq = Uniq::default();

        let metric_conn_count = opentelemetry_api::global::meter_provider()
            .versioned_meter(
                "tx5",
                None::<&'static str>,
                None::<&'static str>,
                Some(vec![opentelemetry_api::KeyValue::new(
                    "state_uniq",
                    state_uniq.to_string(),
                )]),
            )
            .i64_observable_up_down_counter_atomic("tx5.endpoint.conn.count", 0)
            .with_description("Count of open connections managed by endpoint")
            .init()
            .0;

        let meta = StateMeta {
            state_uniq: state_uniq.clone(),
            config,
            conn_limit,
            snd_limit,
            metric_conn_count,
            snd_ident: Arc::new(std::sync::atomic::AtomicU64::new(1)),
        };

        let (state_snd, state_rcv) = tokio::sync::mpsc::unbounded_channel();
        let actor = {
            let meta = meta.clone();
            Actor::new(move |this, rcv| {
                state_task(
                    rcv,
                    state_uniq,
                    StateWeak(this, meta.clone()),
                    meta,
                    StateEvtSnd(state_snd),
                    rcv_limit,
                )
            })
        };

        let weak = StateWeak(actor.weak(), meta.clone());
        tokio::task::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                match weak.upgrade() {
                    None => break,
                    Some(actor) => {
                        if actor.tick_1s().is_err() {
                            break;
                        }
                    }
                }
            }
        });

        Ok((Self(actor, meta), ManyRcv(state_rcv)))
    }

    /// Get a weak version of this State instance.
    pub fn weak(&self) -> StateWeak {
        StateWeak(self.0.weak(), self.1.clone())
    }

    /// Returns `true` if this State is closed.
    pub fn is_closed(&self) -> bool {
        self.0.is_closed()
    }

    /// Shutdown the state management with an error.
    pub fn close(&self, err: std::io::Error) {
        self.0.close(err);
    }

    /// List the ids of current open connections.
    pub fn list_connected(
        &self,
    ) -> impl Future<Output = Result<Vec<Tx5Url>>> + 'static + Send {
        let this = self.clone();
        async move {
            let (s, r) = tokio::sync::oneshot::channel();
            this.0.send(Ok(StateCmd::ListConnected(s)))?;
            r.await.map_err(|_| Error::id("Closed"))?
        }
    }

    /// Establish a new listening connection through given signal server.
    pub fn listener_sig(
        &self,
        sig_url: Tx5Url,
    ) -> impl Future<Output = Result<Tx5Url>> + 'static + Send {
        let this = self.clone();
        async move {
            if !sig_url.is_server() {
                return Err(Error::err(
                    "Invalid tx5 client url, expected signal server url",
                ));
            }
            let (s, r) = tokio::sync::oneshot::channel();
            this.0
                .send(Ok(StateCmd::AssertListenerSig { sig_url, resp: s }))?;
            r.await.map_err(|_| Error::id("Closed"))?
        }
    }

    /// Close down all connections to, fail all outgoing messages to,
    /// and drop all incoming messages from, the given remote id,
    /// for the specified ban time period.
    pub fn ban(&self, rem_id: Id, span: std::time::Duration) {
        let _ = self.0.send(Ok(StateCmd::Ban { rem_id, span }));
    }

    /// Schedule data to be sent out over a channel managed by the state system.
    /// The future will resolve immediately if there is still space
    /// in the outgoing buffer, or once there is again space in the buffer.
    pub fn snd_data<B: bytes::Buf>(
        &self,
        cli_url: Tx5Url,
        data: B,
    ) -> impl Future<Output = Result<()>> + 'static + Send {
        let timestamp = std::time::Instant::now();

        let buf_list = if !cli_url.is_client() {
            Err(Error::err(
                "Invalid tx5 signal server url, expect client url",
            ))
        } else {
            divide_send(&*self.1.config, &self.1.snd_ident, data)
        };

        let meta = self.1.clone();

        let this = self.clone();
        async move {
            tokio::time::timeout(meta.config.max_conn_init(), async move {
                let cli_url = &cli_url;
                let msg_uniq = meta.state_uniq.sub();
                let msg_uniq = &msg_uniq;

                let buf_list = buf_list?;

                let mut resp_list = Vec::with_capacity(buf_list.len());

                for (idx, mut buf) in buf_list.into_iter().enumerate() {
                    let len = buf.len()?;

                    tracing::trace!(%msg_uniq, %len, "snd_data");

                    if meta.snd_limit.available_permits() < len {
                        tracing::warn!(%msg_uniq, %len, "send queue full, waiting for permits");
                    }

                    let send_permit = meta
                        .snd_limit
                        .clone()
                        .acquire_many_owned(len as u32)
                        .await
                        .map_err(Error::err)?;

                    tracing::trace!(%msg_uniq, %idx, %len, "snd_data:got permit");

                    let rem_id = cli_url.id().unwrap();

                    let (s_sent, r_sent) = tokio::sync::oneshot::channel();

                    if let Err(err) = this.0.send(Ok(StateCmd::SendData {
                        msg_uniq: msg_uniq.clone(),
                        rem_id,
                        data: buf,
                        timestamp,
                        send_permit,
                        resp: s_sent,
                        cli_url: cli_url.clone(),
                    })) {
                        tracing::trace!(%msg_uniq, %idx, ?err, "snd_data:complete err");
                        return Err(err);
                    }

                    resp_list.push(async move {
                        match r_sent.await.map_err(|_| Error::id("Closed")) {
                            Ok(r) => match r {
                                Ok(_) => {
                                    tracing::trace!(%msg_uniq, %idx, "snd_data:complete ok");
                                }
                                Err(err) => {
                                    tracing::trace!(%msg_uniq, %idx, ?err, "snd_data:complete err");
                                    return Err(err);
                                }
                            },
                            Err(err) => {
                                tracing::trace!(%msg_uniq, %idx, ?err, "snd_data:complete err");
                                return Err(err);
                            }
                        }
                        Ok(())
                    });
                }

                for resp in resp_list {
                    resp.await?;
                }

                Ok(())
            }).await.map_err(|_| Error::id("Timeout"))?
        }
    }

    /// Send a demo broadcast to every connected signal server.
    /// Warning, if demo mode is not enabled on these servers, this
    /// could result in a ban.
    pub fn snd_demo(&self) -> Result<()> {
        self.0.send(Ok(StateCmd::SndDemo))
    }

    /// Get stats.
    pub fn stats(
        &self,
    ) -> impl Future<Output = Result<serde_json::Value>> + 'static + Send {
        let this = self.clone();
        async move {
            let (s, r) = tokio::sync::oneshot::channel();
            this.0.send(Ok(StateCmd::Stats(s)))?;
            r.await.map_err(|_| Error::id("Shutdown"))?
        }
    }

    // -- //

    fn tick_1s(&self) -> Result<()> {
        self.0.send(Ok(StateCmd::Tick1s))
    }

    pub(crate) fn track_sig(&self, rem_id: Id, ty: &'static str, bytes: usize) {
        let _ = self.0.send(Ok(StateCmd::TrackSig { rem_id, ty, bytes }));
    }

    pub(crate) fn publish(&self, evt: StateEvt) {
        let _ = self.0.send(Ok(StateCmd::Publish { evt }));
    }

    pub(crate) fn sig_connected(&self, cli_url: Tx5Url) {
        let _ = self.0.send(Ok(StateCmd::SigConnected { cli_url }));
    }

    pub(crate) fn fetch_for_send(
        &self,
        conn: ConnStateWeak,
        rem_id: Id,
    ) -> Result<()> {
        self.0.send(Ok(StateCmd::FetchForSend { conn, rem_id }))
    }

    pub(crate) fn in_offer(
        &self,
        sig_url: Tx5Url,
        rem_id: Id,
        data: BackBuf,
    ) -> Result<()> {
        self.0.send(Ok(StateCmd::InOffer {
            sig_url,
            rem_id,
            data,
        }))
    }

    pub(crate) fn in_demo(&self, sig_url: Tx5Url, rem_id: Id) -> Result<()> {
        self.0.send(Ok(StateCmd::InDemo { sig_url, rem_id }))
    }

    pub(crate) fn cache_ice(&self, rem_id: Id, ice: BackBuf) -> Result<()> {
        self.0.send(Ok(StateCmd::CacheIce { rem_id, ice }))
    }

    pub(crate) fn get_cached_ice(&self, rem_id: Id) -> Result<()> {
        self.0.send(Ok(StateCmd::GetCachedIce { rem_id }))
    }

    pub(crate) fn close_sig(
        &self,
        sig_url: Tx5Url,
        sig: SigStateWeak,
        err: std::io::Error,
    ) {
        let _ = self.0.send(Ok(StateCmd::CloseSig { sig_url, sig, err }));
    }

    pub(crate) fn conn_ready(&self, cli_url: Tx5Url) {
        let _ = self.0.send(Ok(StateCmd::ConnReady { cli_url }));
    }

    pub(crate) fn close_conn(
        &self,
        rem_id: Id,
        conn: ConnStateWeak,
        err: std::io::Error,
    ) {
        let _ = self.0.send(Ok(StateCmd::CloseConn { rem_id, conn, err }));
    }
}
