use crate::deps::lair_keystore_api;
use crate::deps::sodoken;
use crate::BackBuf;
use crate::{state2::*, Config2};
use lair_keystore_api::prelude::*;

use tx5_core::{Error, Id, Result};

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct OutLimit(HashMap<Id, Arc<tokio::sync::Semaphore>>);

impl OutLimit {
    pub fn get(
        &mut self,
        config: &Config2,
        id: Id,
    ) -> Arc<tokio::sync::Semaphore> {
        self.0
            .entry(id)
            .or_insert_with(|| {
                Arc::new(tokio::sync::Semaphore::new(
                    config.byte_count_max as usize,
                ))
            })
            .clone()
    }
}

/// Tx5 v2 endpoint.
pub struct Ep2 {
    config: Arc<Config2>,
    this_id: Id,
    cmd_send: tokio::sync::mpsc::Sender<State2Cmd>,
    state_task: tokio::task::JoinHandle<()>,
    tick_task: tokio::task::JoinHandle<()>,
    out_limit: Arc<Mutex<OutLimit>>,
}

impl Drop for Ep2 {
    fn drop(&mut self) {
        self.state_task.abort();
        self.tick_task.abort();
    }
}

impl Ep2 {
    /// Construct a new tx5 v2 endpoint.
    pub async fn new(config: Arc<Config2>) -> Self {
        let lair_tag: Arc<str> =
            rand_utf8::rand_utf8(&mut rand::thread_rng(), 32).into();

        let passphrase = sodoken::BufRead::new_no_lock(
            rand_utf8::rand_utf8(&mut rand::thread_rng(), 32).as_bytes(),
        );

        // this is a memory keystore,
        // so weak persistence security is okay,
        // since it will not be persisted.
        // The private keys will still be mem_locked
        // so they shouldn't be swapped to disk.
        let keystore_config = PwHashLimits::Minimum
            .with_exec(|| LairServerConfigInner::new("/", passphrase.clone()))
            .await
            .unwrap();

        let lair_keystore = PwHashLimits::Minimum
            .with_exec(|| {
                lair_keystore_api::in_proc_keystore::InProcKeystore::new(
                    Arc::new(keystore_config),
                    lair_keystore_api::mem_store::create_mem_store_factory(),
                    passphrase,
                )
            })
            .await
            .unwrap();

        let lair_client = lair_keystore.new_client().await.unwrap();

        let seed = lair_client
            .new_seed(lair_tag.clone(), None, false)
            .await
            .unwrap();

        let this_id = Id(*seed.x25519_pub_key.0);

        let (cmd_send, cmd_recv) = tokio::sync::mpsc::channel(1024);

        let state_task = tokio::task::spawn(state_task(
            config.clone(),
            this_id,
            lair_tag,
            lair_keystore,
            lair_client,
            cmd_send.clone(),
            cmd_recv,
        ));

        let tick_task = {
            let cmd_send = cmd_send.clone();
            tokio::task::spawn(async move {
                let mut int =
                    tokio::time::interval(std::time::Duration::from_secs(2));
                int.set_missed_tick_behavior(
                    tokio::time::MissedTickBehavior::Delay,
                );
                loop {
                    int.tick().await;
                    if cmd_send.send(State2Cmd::Tick).await.is_err() {
                        break;
                    }
                }
            })
        };

        Self {
            config,
            this_id,
            cmd_send,
            state_task,
            tick_task,
            out_limit: Arc::new(Mutex::new(OutLimit::default())),
        }
    }

    /// Establish a listening connection to a signal server,
    /// from which we can accept incoming remote connections.
    /// Returns the client url at which this endpoint may now be addressed.
    pub async fn listen(&self, sig_url: SigUrl) -> Result<PeerUrl> {
        if !sig_url.is_server() {
            return Err(Error::id("ExpectedSignalServerUrl"));
        }

        let peer_url = sig_url.to_client(self.this_id);

        self.cmd_send
            .send(State2Cmd::SigAssert {
                sig_url,
                is_listening: true,
            })
            .await
            .map_err(|_| Error::str("Endpoint closed when trying to assert signal connection"))?;

        Ok(peer_url)
    }

    /// Send data to a remote on this tx5 endpoint.
    /// The future returned from this method will resolve when
    /// the data is handed off to our networking backend.
    pub fn send(
        &self,
        peer_url: PeerUrl,
        mut data: BackBuf,
    ) -> impl std::future::Future<Output = Result<()>> + 'static + Send {
        let cmd_send = self.cmd_send.clone();
        let config = self.config.clone();
        let out_limit = self.out_limit.clone();

        async move {
            let len = data.len()?;
            if len > 16 * 1024 {
                return Err(Error::id("DataTooLarge"));
            }

            if !peer_url.is_client() {
                return Err(Error::id("InvalidPeerUrl"));
            }
            let sig_url = peer_url.to_server();
            let peer_id = peer_url.id().unwrap();

            let limit = out_limit.lock().unwrap().get(&config, peer_id);

            let permit = limit
                .acquire_many_owned(len as u32)
                .await
                .map_err(|_| Error::id("BytesOutMaxLimitError"))?;

            let (s, r) = tokio::sync::oneshot::channel();

            let cmd = State2Cmd::SendMsg {
                sig_url,
                peer_id,
                permit,
                data,
                resp: Arc::new(Mutex::new(Some(s))),
            };

            cmd_send
                .send(cmd)
                .await
                .map_err(|_| Error::str("Endpoint closed when trying to forward message for send"))?;

            r.await.map_err(|_| Error::id("Send message result responder dropped"))?
        }
    }
}

async fn state_task(
    config: Arc<Config2>,
    this_id: Id,
    lair_tag: Arc<str>,
    _lair_keystore: lair_keystore_api::in_proc_keystore::InProcKeystore,
    lair_client: LairClient,
    cmd_send: tokio::sync::mpsc::Sender<State2Cmd>,
    mut cmd_recv: tokio::sync::mpsc::Receiver<State2Cmd>,
) {
    let mut state = State2::new(config, this_id);

    let mut evt_list = VecDeque::new();

    while let Some(cmd) = cmd_recv.recv().await {
        state.cmd(cmd, &mut evt_list);

        while let Some(evt) = evt_list.pop_front() {
            match evt {
                State2Evt::SigCreate { sig_url, sig_uniq } => {
                    let _sig_task = tokio::task::spawn(sig_task(
                        cmd_send.clone(),
                        sig_url,
                        sig_uniq,
                        lair_tag.clone(),
                        lair_client.clone(),
                    ));
                }
                State2Evt::SigClose {
                    sig_url: _,
                    sig_uniq: _,
                    err: _,
                } => {}
                State2Evt::PeerCreate {
                    sig_url,
                    sig_uniq,
                    peer_id,
                    peer_uniq,
                    ice_servers,
                } => {
                    let _peer_task = tokio::task::spawn(peer_task(
                        cmd_send.clone(),
                        sig_url,
                        sig_uniq,
                        peer_id,
                        peer_uniq,
                        ice_servers,
                    ));
                }
                State2Evt::PeerClose {
                    peer_id: _,
                    peer_uniq: _,
                    err: _,
                } => {}
            }
        }
    }
}

async fn sig_task(
    cmd_send: tokio::sync::mpsc::Sender<State2Cmd>,
    sig_url: SigUrl,
    sig_uniq: SigUniq,
    lair_tag: Arc<str>,
    lair_client: LairClient,
) {
    let err = sig_task_err(
        cmd_send.clone(),
        sig_url.clone(),
        sig_uniq,
        lair_tag,
        lair_client,
    )
    .await
    .err()
    .unwrap_or_else(|| Error::id("SignalTaskEnded"))
    .into();

    let _ = cmd_send
        .send(State2Cmd::SigClose {
            sig_url,
            sig_uniq,
            err,
        })
        .await;
}

async fn sig_task_err(
    cmd_send: tokio::sync::mpsc::Sender<State2Cmd>,
    sig_url: SigUrl,
    sig_uniq: SigUniq,
    lair_tag: Arc<str>,
    lair_client: LairClient,
) -> Result<()> {
    let (sig, mut sig_rcv) = tx5_signal::Cli::builder()
        .with_lair_client(lair_client)
        .with_lair_tag(lair_tag)
        .with_url(sig_url.to_string().parse().unwrap())
        .build()
        .await?;

    let cli_url = PeerUrl::new(sig.local_addr())?;
    let this_id = cli_url.id().unwrap();

    let ice_servers = sig.ice_servers();

    cmd_send
        .send(State2Cmd::SigCmd {
            sig_url: sig_url.clone(),
            sig_uniq,
            sig_cmd: SigCmd::Open {
                this_id,
                ice_servers,
            },
        })
        .await
        .map_err(|_| Error::str("Endpoint closed when reporting sig open"))?;

    while let Some(msg) = sig_rcv.recv().await {
        println!("SIG_RECV: {msg:?}");
    }

    Ok(())
}

async fn peer_task(
    cmd_send: tokio::sync::mpsc::Sender<State2Cmd>,
    sig_url: SigUrl,
    sig_uniq: SigUniq,
    peer_id: Id,
    peer_uniq: PeerUniq,
    ice_servers: Arc<serde_json::Value>,
) {
    let err = peer_task_err(
        cmd_send.clone(),
        sig_url.clone(),
        sig_uniq,
        peer_id,
        peer_uniq,
        ice_servers,
    )
    .await
    .err()
    .unwrap_or_else(|| Error::id("PeerTaskEnded"))
    .into();

    let _ = cmd_send
        .send(State2Cmd::SigCmd {
            sig_url,
            sig_uniq,
            sig_cmd: SigCmd::PeerClose {
                peer_id,
                peer_uniq,
                err,
            },
        })
        .await;
}

async fn peer_task_err(
    _cmd_send: tokio::sync::mpsc::Sender<State2Cmd>,
    _sig_url: SigUrl,
    _sig_uniq: SigUniq,
    _peer_id: Id,
    _peer_uniq: PeerUniq,
    ice_servers: Arc<serde_json::Value>,
) -> Result<()> {
    let _peer_config = BackBuf::from_json(ice_servers)?;

    /*
    let (peer, peer_recv) = tx5_go_pion::PeerConnection::new(
        peer_config.imp.buf,
        seed.rcv_limit().clone(),
    )
    .await?;
    */

    Ok(())
}

#[cfg(test)]
mod ep2_tests {
    use super::*;

    fn init_tracing() {
        let subscriber = tracing_subscriber::FmtSubscriber::builder()
            .with_env_filter(
                tracing_subscriber::filter::EnvFilter::from_default_env(),
            )
            .with_file(true)
            .with_line_number(true)
            .finish();
        let _ = tracing::subscriber::set_global_default(subscriber);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn ep2_sanity() {
        init_tracing();

        let mut srv_config = tx5_signal_srv::Config::default();
        srv_config.port = 0;

        let (srv_driver, addr_list, _) =
            tx5_signal_srv::exec_tx5_signal_srv(srv_config).unwrap();
        let sig_task = tokio::task::spawn(srv_driver);
        let sig_port = addr_list.get(0).unwrap().port();
        let sig_url =
            SigUrl::new(format!("ws://localhost:{}", sig_port)).unwrap();
        println!("sig_url: {sig_url}");

        let ep1 = Ep2::new(Arc::new(Config2::default())).await;
        let cli_url1 = ep1.listen(sig_url.clone()).await.unwrap();
        println!("cli_url1: {cli_url1}");

        let ep2 = Ep2::new(Arc::new(Config2::default())).await;
        let cli_url2 = ep2.listen(sig_url).await.unwrap();
        println!("cli_url2: {cli_url2}");

        ep1.send(cli_url2, BackBuf::from_slice(b"hello").unwrap())
            .await
            .unwrap();

        sig_task.abort();
    }
}
