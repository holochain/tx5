#![deny(missing_docs)]
#![deny(unsafe_code)]
#![deny(warnings)]
#![doc = tx5_core::__doc_header!()]
//! # tx5-pipe
//!
//! Holochain WebRTC p2p cli tool.

use crate::boot::BootEntry;
use lair_keystore_api::prelude::*;

/// Current Tx5Pipe version.
pub const TX5_PIPE_VERSION: &str = env!("CARGO_PKG_VERSION");

const HELP_TEXT: &str = r#"
# -- Begin Tx5Pipe Usage Info -- #
# # Request Messages (received on stdin):
# 'app_reg cmd_id base64_app_hash' - register your app hash
# 'sig_reg cmd_id sig_url'         - connect to a signal server
# 'boot_reg cmd_id boot_url data'  - register with a bootstrap server
# 'send cmd_id rem_url data'       - send outgoing data to a peer
# # Response Messages (sent on stdout):
# '@help version info'             - this help info
# '@error cmd_id code text'        - error response to a request
# '@app_reg_ok cmd_id'             - okay response to an app_reg
# '@sig_reg_ok cmd_id cli_url'     - okay response to a sig_reg
# '@boot_reg_ok cmd_id'            - okay response to a boot_reg
# '@send_ok cmd_id'                - okay response to a send
# '@recv rem_url data'             - receive incoming peer data
# -- End Tx5Pipe Usage Info -- #"#;

pub mod boot;

use std::sync::Arc;
use tx5_core::pipe_ipc::*;
use tx5_core::Tx5Url;

/// Implement this to handle events generated by Tx5Pipe.
pub trait Tx5PipeHandler: 'static + Send + Sync {
    /// A fatal error was generated, the pipe is closed.
    fn fatal(&self, error: std::io::Error);

    /// A pipe response was generated. This function should return:
    /// - `true` if we should keep running, or
    /// - `false` if we should shut down the pipe.
    fn pipe(&self, response: Tx5PipeResponse) -> bool;
}

type DynTx5PipeHandler = Arc<dyn Tx5PipeHandler + 'static + Send + Sync>;

#[derive(Default)]
struct BootCache {
    cache: std::collections::HashMap<[u8; 32], BootEntry>,
}

impl BootCache {
    fn incoming(
        &mut self,
        list: Vec<BootEntry>,
    ) -> Vec<([u8; 32], Option<String>, Vec<u8>)> {
        use std::collections::hash_map::Entry;
        let mut out = Vec::new();

        let now = std::time::SystemTime::now();

        self.cache.retain(|_k, a| a.expires_at > now);

        for boot_entry in list {
            if boot_entry.expires_at <= now {
                continue;
            }

            match self.cache.entry(boot_entry.pub_key) {
                Entry::Occupied(mut e) => {
                    if boot_entry.signed_at > e.get().signed_at {
                        out.push((
                            boot_entry.pub_key,
                            boot_entry.cli_url.clone(),
                            boot_entry.data.clone(),
                        ));
                        e.insert(boot_entry);
                    }
                }
                Entry::Vacant(e) => {
                    out.push((
                        boot_entry.pub_key,
                        boot_entry.cli_url.clone(),
                        boot_entry.data.clone(),
                    ));
                    e.insert(boot_entry);
                }
            }
        }

        out
    }
}

/// The Tx5Pipe struct.
pub struct Tx5Pipe {
    ep: tx5::Ep,
    hnd: DynTx5PipeHandler,
    cur_cli_url: Arc<std::sync::Mutex<Option<String>>>,
    cur_app_hash: Arc<std::sync::Mutex<Option<[u8; 32]>>>,
    cur_boot_url: Arc<std::sync::Mutex<Option<String>>>,
    cur_boot_data: Arc<std::sync::Mutex<Option<Vec<u8>>>>,
    recv_task: tokio::task::JoinHandle<()>,
    boot_task: Arc<std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>>,
}

impl Drop for Tx5Pipe {
    fn drop(&mut self) {
        self.recv_task.abort();
        if let Some(boot_task) = self.boot_task.lock().unwrap().take() {
            boot_task.abort();
        }
    }
}

impl Tx5Pipe {
    /// Construct a new Tx5Pipe instance.
    pub async fn new<H: Tx5PipeHandler>(hnd: H) -> std::io::Result<Arc<Self>> {
        let (ep, mut recv) = tx5::Ep::new().await?;

        let hnd: DynTx5PipeHandler = Arc::new(hnd);

        if !hnd.pipe(Tx5PipeResponse::Tx5PipeHelp {
            version: TX5_PIPE_VERSION.to_string(),
            info: HELP_TEXT.to_string(),
        }) {
            return Err(tx5_core::Error::err(
                "instructed to shutdown by recv handler",
            ));
        }

        let recv_task = {
            let hnd = hnd.clone();
            tokio::task::spawn(async move {
                while let Some(evt) = recv.recv().await {
                    let evt = match evt {
                        Err(err) => {
                            hnd.fatal(err);
                            return;
                        }
                        Ok(evt) => evt,
                    };
                    if let tx5::EpEvt::Data {
                        rem_cli_url,
                        data,
                        permit: _permit,
                    } = evt
                    {
                        if !hnd.pipe(Tx5PipeResponse::Recv {
                            rem_url: rem_cli_url.to_string(),
                            data,
                        }) {
                            hnd.fatal(tx5_core::Error::err(
                                "instructed to shutdown by recv handler",
                            ));
                            return;
                        }
                    }
                }
            })
        };

        Ok(Arc::new(Self {
            ep,
            hnd,
            cur_cli_url: Arc::new(std::sync::Mutex::new(None)),
            cur_app_hash: Arc::new(std::sync::Mutex::new(None)),
            cur_boot_url: Arc::new(std::sync::Mutex::new(None)),
            cur_boot_data: Arc::new(std::sync::Mutex::new(None)),
            recv_task,
            boot_task: Arc::new(std::sync::Mutex::new(None)),
        }))
    }

    /// Send a pipe request into the Tx5Pipe instance.
    pub fn pipe(self: &Arc<Self>, req: Tx5PipeRequest) {
        let this = self.clone();
        tokio::task::spawn(async move {
            match req {
                Tx5PipeRequest::AppReg { cmd_id, app_hash } => {
                    this.handle_resp(
                        cmd_id.clone(),
                        this.app_reg(cmd_id, app_hash).await,
                    );
                }
                Tx5PipeRequest::SigReg { cmd_id, sig_url } => {
                    this.handle_resp(
                        cmd_id.clone(),
                        this.sig_reg(cmd_id, sig_url).await,
                    );
                }
                Tx5PipeRequest::BootReg {
                    cmd_id,
                    boot_url,
                    data,
                } => {
                    this.handle_resp(
                        cmd_id.clone(),
                        this.boot_reg(cmd_id, boot_url, data).await,
                    );
                }
                Tx5PipeRequest::Send {
                    cmd_id,
                    rem_url,
                    data,
                } => {
                    this.handle_resp(
                        cmd_id.clone(),
                        this.send(cmd_id, rem_url, data).await,
                    );
                }
            }
        });
    }

    fn handle_resp(
        &self,
        cmd_id: String,
        rsp: std::io::Result<Tx5PipeResponse>,
    ) {
        let rsp = match rsp {
            Ok(rsp) => rsp,
            Err(err) => Tx5PipeResponse::Error {
                cmd_id,
                code: 0,
                text: format!("{err:?}"),
            },
        };
        if !self.hnd.pipe(rsp) {
            todo!("handle pipe close in responder"); // TODO - FIXME
        }
    }

    async fn check_do_bootstrap(&self) -> std::io::Result<()> {
        let cur_app_hash = *self.cur_app_hash.lock().unwrap();
        let cur_cli_url = self.cur_cli_url.lock().unwrap().clone();
        let cur_boot_url = self.cur_boot_url.lock().unwrap().clone();

        if let (Some(app_hash), Some(cli_url), Some(boot_url)) =
            (cur_app_hash, cur_cli_url, cur_boot_url)
        {
            let lair_tag = self.ep.get_config().lair_tag().clone();
            let ed25519_pub_key = match self
                .ep
                .get_config()
                .lair_client()
                .get_entry(lair_tag)
                .await?
            {
                LairEntryInfo::Seed {
                    seed_info:
                        SeedInfo {
                            ed25519_pub_key, ..
                        },
                    ..
                } => ed25519_pub_key,
                _ => return Err(tx5_core::Error::id("InvalidLairEntry")),
            };

            let pub_key: [u8; 32] = *ed25519_pub_key.0;

            let signed_at = std::time::SystemTime::now();
            let expires_at =
                signed_at + std::time::Duration::from_secs(60) * 20;

            let data = self
                .cur_boot_data
                .lock()
                .unwrap()
                .clone()
                .unwrap_or_default();

            let entry = boot::BootEntry {
                pub_key,
                app_hash,
                cli_url: Some(cli_url),
                signed_at,
                expires_at,
                data,
            };

            let enc1: Arc<[u8]> = entry.encode()?.into_boxed_slice().into();

            let sig = self
                .ep
                .get_config()
                .lair_client()
                .sign_by_pub_key(ed25519_pub_key, None, enc1.clone())
                .await?;
            let signature: [u8; 64] = *sig.0;

            let enc2 = entry.sign(&enc1, &signature)?;

            boot::boot_put(&boot_url, enc2).await?;

            {
                let mut boot_task_lock = self.boot_task.lock().unwrap();
                if boot_task_lock.is_none() {
                    let cur_app_hash = self.cur_app_hash.clone();
                    let hnd = self.hnd.clone();
                    *boot_task_lock = Some(tokio::task::spawn(async move {
                        let mut delay_s = 10;
                        let mut boot_cache = BootCache::default();
                        loop {
                            let app_hash = *cur_app_hash.lock().unwrap();
                            if let Some(app_hash) = app_hash {
                                if let Ok(list) =
                                    boot::boot_random(&boot_url, &app_hash)
                                        .await
                                {
                                    for (rem_pub_key, rem_url, data) in
                                        boot_cache.incoming(list)
                                    {
                                        if !hnd.pipe(
                                            Tx5PipeResponse::BootRecv {
                                                rem_pub_key,
                                                rem_url,
                                                data: Box::new(
                                                    std::io::Cursor::new(data),
                                                ),
                                            },
                                        ) {
                                            todo!("handle pipe close in responder");
                                            // TODO - FIXME
                                        }
                                    }
                                }
                            }

                            tokio::time::sleep(std::time::Duration::from_secs(
                                delay_s,
                            ))
                            .await;
                            delay_s *= 2;
                            if delay_s > 60 {
                                delay_s = 60;
                            }
                        }
                    }));
                }
            }
        }

        Ok(())
    }

    async fn app_reg(
        &self,
        cmd_id: String,
        app_hash: [u8; 32],
    ) -> std::io::Result<Tx5PipeResponse> {
        *self.cur_app_hash.lock().unwrap() = Some(app_hash);
        self.check_do_bootstrap().await?;
        Ok(Tx5PipeResponse::AppRegOk { cmd_id })
    }

    async fn sig_reg(
        &self,
        cmd_id: String,
        sig_url: String,
    ) -> std::io::Result<Tx5PipeResponse> {
        let sig_url = Tx5Url::new(sig_url)?;
        let cli_url = self.ep.listen(sig_url).await?;
        *self.cur_cli_url.lock().unwrap() = Some(cli_url.to_string());
        self.check_do_bootstrap().await?;
        Ok(Tx5PipeResponse::SigRegOk {
            cmd_id,
            cli_url: cli_url.to_string(),
        })
    }

    async fn boot_reg(
        &self,
        cmd_id: String,
        boot_url: String,
        data: Box<dyn bytes::Buf + Send>,
    ) -> std::io::Result<Tx5PipeResponse> {
        use tx5::BytesBufExt;

        boot::boot_ping(&boot_url).await?;
        *self.cur_boot_url.lock().unwrap() = Some(boot_url);
        let data = data.to_vec()?;
        if data.is_empty() {
            *self.cur_boot_data.lock().unwrap() = None;
        } else {
            *self.cur_boot_data.lock().unwrap() = Some(data);
        }
        self.check_do_bootstrap().await?;
        Ok(Tx5PipeResponse::BootRegOk { cmd_id })
    }

    async fn send(
        &self,
        cmd_id: String,
        rem_url: String,
        data: Box<dyn bytes::Buf + Send>,
    ) -> std::io::Result<Tx5PipeResponse> {
        let rem_url = Tx5Url::new(rem_url)?;
        self.ep.send(rem_url, data).await?;
        Ok(Tx5PipeResponse::SendOk { cmd_id })
    }
}
