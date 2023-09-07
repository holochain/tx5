#![deny(missing_docs)]
#![deny(unsafe_code)]
#![deny(warnings)]
#![doc = tx5_core::__doc_header!()]
//! # tx5-pipe
//!
//! Holochain WebRTC p2p cli tool.

use lair_keystore_api::prelude::*;

/// Current Tx5Pipe version.
pub const TX5_PIPE_VERSION: &str = env!("CARGO_PKG_VERSION");

const HELP_TEXT: &str = r#"
# -- Begin Tx5Pipe Usage Info -- #
#
# Requests / responses are terminated by newlines.
# Fields / parameters are separated by spaces.
# Use quotes if your field contains newlines.
# Use `6|binary` format for efficiency or if you need
# to include binary or utf8 characters.
#
# Example:
#
# send "my command id" wss://my.server/tx5-ws/rem.id `4|🙃`
#
# # Request Messages (received on stdin):
#
# * sig_reg           - connect to a signal server
#   - cmd_id          - command identifier
#   - sig_url         - signal server url (wss://...)
#
# * boot_reg          - register with a bootstrap server
#   - cmd_id          - command identifier
#   - boot_url        - bootstrap server url (https://...)
#   - app_hash        - base64 encoded application hash
#   - cli_url         - signal client url (wss://.../tx5-ws/...)
#   - data            - any binary meta-data
#
# * boot_query        - request a list of peers from bootstrap
#   - cmd_id          - command identifier
#   - boot_url        - bootstrap server url (https://...)
#   - app_hash        - base64 encoded application hash
#
# * send              - send outgoing data to a peer
#   - cmd_id          - command identifier
#   - rum_url         - signal client url (wss://.../tx5-ws/...)
#   - data            - any data to send to remote peer
#
# # Response Messages (sent on stdout):
#
# * @help version info             - this help info
# * @error cmd_id code text        - error response to a request
# * @sig_reg_ok cmd_id cli_url     - okay response to a sig_reg
# * @boot_reg_ok cmd_id            - okay response to a boot_reg
# * @boot_query_resp cmd_id pub_key rem_url expires_at_unix_epoch_s data
#                                  - bootstrap query peer response
# * @boot_query_ok cmd_id          - okay response to a boot_reg
# * @send_ok cmd_id                - okay response to a send
# * @recv rem_url data             - receive incoming peer data
#
# -- End Tx5Pipe Usage Info -- #"#;

pub mod boot;
use boot::BootEntry;

use std::collections::HashMap;
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

/// The Tx5Pipe struct.
pub struct Tx5Pipe {
    ep: tx5::Ep,
    hnd: DynTx5PipeHandler,
    recv_task: tokio::task::JoinHandle<()>,
    #[allow(clippy::type_complexity)]
    boot_reg: Arc<std::sync::Mutex<HashMap<[u8; 32], (String, Vec<u8>)>>>,
}

impl Drop for Tx5Pipe {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl Tx5Pipe {
    /// Construct a new Tx5Pipe instance.
    pub async fn new<H: Tx5PipeHandler>(hnd: H) -> std::io::Result<Arc<Self>> {
        let (ep, mut recv) = tx5::Ep::new().await?;

        let hnd: DynTx5PipeHandler = Arc::new(hnd);

        if !hnd.pipe(Tx5PipeResponse::Help {
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
            recv_task,
            boot_reg: Arc::new(std::sync::Mutex::new(HashMap::new())),
        }))
    }

    /// Spawn a Tx5Pipe in-process and return a control for
    /// communicating through it.
    pub async fn spawn_in_process<
        H: tx5_pipe_control::Tx5PipeControlHandler,
    >(
        control_handler: H,
    ) -> std::io::Result<tx5_pipe_control::Tx5PipeControl> {
        tx5_pipe_control::Tx5PipeControl::new(control_handler, |ingest| async move {
            struct Hnd {
                ingest: tx5_pipe_control::control_impl::Tx5PipeControlIngest,
            }

            impl Tx5PipeHandler for Hnd {
                fn fatal(&self, _error: std::io::Error) {
                    todo!()
                }

                fn pipe(&self, response: Tx5PipeResponse) -> bool {
                    self.ingest.handle_response(response);
                    // TODO always true?
                    true
                }
            }

            let pipe = Tx5Pipe::new(Hnd { ingest }).await?;

            struct Impl {
                pipe: Arc<Tx5Pipe>,
            }

            impl tx5_pipe_control::control_impl::Tx5PipeControlImpl for Impl {
                fn shutdown(&self, _error: tx5_core::Error) {}

                fn request(&self, req: Tx5PipeRequest) {
                    self.pipe.pipe(req);
                }
            }

            Ok(Impl { pipe })
        })
        .await
    }

    /// Shutdown this pipe instance.
    /// You can await the join handle, or not at your option.
    pub fn shutdown(&self) -> tokio::task::JoinHandle<()> {
        self.recv_task.abort();
        let bootstrap = std::mem::take(&mut *self.boot_reg.lock().unwrap());
        let ep = self.ep.clone();
        tokio::task::spawn(async move {
            for (app_hash, (boot_url, data)) in bootstrap {
                let _ =
                    Self::put_bootstrap(&ep, &boot_url, app_hash, None, data)
                        .await;
            }
        })
    }

    /// Send a pipe request into the Tx5Pipe instance.
    pub fn pipe(self: &Arc<Self>, req: Tx5PipeRequest) {
        let this = self.clone();
        tokio::task::spawn(async move {
            match req {
                Tx5PipeRequest::SigReg { cmd_id, sig_url } => {
                    this.handle_resp(
                        cmd_id.clone(),
                        this.sig_reg(cmd_id, sig_url).await,
                    );
                }
                Tx5PipeRequest::BootReg {
                    cmd_id,
                    boot_url,
                    app_hash,
                    cli_url,
                    data,
                } => {
                    this.handle_resp(
                        cmd_id.clone(),
                        this.boot_reg(
                            cmd_id, boot_url, app_hash, cli_url, data,
                        )
                        .await,
                    );
                }
                Tx5PipeRequest::BootQuery {
                    cmd_id,
                    boot_url,
                    app_hash,
                } => {
                    this.handle_resp(
                        cmd_id.clone(),
                        this.boot_query(cmd_id, boot_url, app_hash).await,
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

    async fn sig_reg(
        &self,
        cmd_id: String,
        sig_url: String,
    ) -> std::io::Result<Tx5PipeResponse> {
        let sig_url = Tx5Url::new(sig_url)?;
        let cli_url = self.ep.listen(sig_url).await?;
        Ok(Tx5PipeResponse::SigRegOk {
            cmd_id,
            cli_url: cli_url.to_string(),
        })
    }

    async fn put_bootstrap(
        ep: &tx5::Ep,
        boot_url: &str,
        app_hash: [u8; 32],
        cli_url: Option<String>,
        data: Vec<u8>,
    ) -> std::io::Result<()> {
        let lair_tag = ep.get_config().lair_tag().clone();
        let ed25519_pub_key =
            match ep.get_config().lair_client().get_entry(lair_tag).await? {
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
        let expires_at = signed_at + std::time::Duration::from_secs(60) * 20;

        let entry = BootEntry {
            pub_key,
            app_hash,
            cli_url,
            signed_at,
            expires_at,
            data: data.clone(),
        };

        let enc1: Arc<[u8]> = entry.encode()?.into_boxed_slice().into();

        let sig = ep
            .get_config()
            .lair_client()
            .sign_by_pub_key(ed25519_pub_key, None, enc1.clone())
            .await?;
        let signature: [u8; 64] = *sig.0;

        let enc2 = entry.sign(&enc1, &signature)?;

        boot::boot_put(boot_url, enc2).await?;

        Ok(())
    }

    async fn boot_reg(
        &self,
        cmd_id: String,
        boot_url: String,
        app_hash: [u8; 32],
        cli_url: Option<String>,
        data: Box<dyn bytes::Buf + Send>,
    ) -> std::io::Result<Tx5PipeResponse> {
        use tx5::BytesBufExt;

        let data = data.to_vec()?;

        let is_remove = cli_url.is_none();

        Self::put_bootstrap(
            &self.ep,
            &boot_url,
            app_hash,
            cli_url,
            data.clone(),
        )
        .await?;

        if is_remove {
            self.boot_reg.lock().unwrap().remove(&app_hash);
        } else {
            self.boot_reg
                .lock()
                .unwrap()
                .insert(app_hash, (boot_url, data));
        }

        Ok(Tx5PipeResponse::BootRegOk { cmd_id })
    }

    async fn boot_query(
        &self,
        cmd_id: String,
        boot_url: String,
        app_hash: [u8; 32],
    ) -> std::io::Result<Tx5PipeResponse> {
        let list = boot::boot_random(&boot_url, &app_hash).await?;

        for entry in list {
            if !self.hnd.pipe(Tx5PipeResponse::BootQueryResp {
                cmd_id: cmd_id.clone(),
                rem_pub_key: entry.pub_key,
                rem_url: entry.cli_url,
                expires_at_s: entry
                    .expires_at
                    .duration_since(std::time::SystemTime::UNIX_EPOCH)
                    .map_err(tx5_core::Error::err)?
                    .as_secs(),
                data: Box::new(std::io::Cursor::new(entry.data)),
            }) {
                todo!("handle pipe close in responder");
                // TODO - FIXME
            }
        }

        Ok(Tx5PipeResponse::BootQueryOk { cmd_id })
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
