//! tx5-pipe client types.

use std::future::Future;
use std::io::Result;
use std::sync::Arc;
use tx5_core::pipe_ipc::*;
use tx5_core::Error;

/// Use these types when writing your own tx5-pipe client impl.
pub mod client_impl {
    use super::*;

    /// Use this to report messages when writing your own tx5-pipe client impl.
    pub struct Tx5PipeClientIngest {
        pub(crate) resp_cache: Arc<std::sync::Mutex<RespCache>>,
        pub(crate) client_handler: DynTx5PipeClientHandler,
    }

    impl Tx5PipeClientIngest {
        /// Report a message from the server.
        pub fn handle_response(&self, resp: Tx5PipeResponse) {
            if let Some(cmd_id) = resp.get_cmd_id() {
                self.resp_cache.lock().unwrap().resp(cmd_id.clone(), resp);
                return;
            }

            match resp {
                Tx5PipeResponse::Error { .. }
                | Tx5PipeResponse::AppRegOk { .. }
                | Tx5PipeResponse::SigRegOk { .. }
                | Tx5PipeResponse::BootRegOk { .. }
                | Tx5PipeResponse::SendOk { .. } => unreachable!(),
                Tx5PipeResponse::Tx5PipeHelp { .. } => (),
                Tx5PipeResponse::Recv { rem_url, data } => {
                    self.client_handler.recv(rem_url, data);
                }
                Tx5PipeResponse::BootRecv {
                    rem_pub_key,
                    rem_url,
                    data,
                } => {
                    self.client_handler.boot_recv(rem_pub_key, rem_url, data);
                }
            }
        }
    }

    /// Implement this to write your own tx5-pipe client impl.
    pub trait Tx5PipeClientImpl: 'static + Send + Sync {
        /// Shut down the pipe.
        fn shutdown(&self, error: Error);

        /// Send a request to the server.
        fn request(&self, req: Tx5PipeRequest);
    }

    pub(crate) type DynTx5PipeClientImpl =
        Arc<dyn Tx5PipeClientImpl + 'static + Send + Sync>;
}
pub(crate) use client_impl::*;

/// Implement this to handle unsolicited incoming tx5-pipe messages.
pub trait Tx5PipeClientHandler: 'static + Send + Sync {
    /// Incoming message from remote.
    fn recv(&self, rem_url: String, data: Box<dyn bytes::Buf + Send>);

    /// Incoming bootstrap notice of peer availability
    /// (or unavailability if rem_url is None).
    fn boot_recv(
        &self,
        rem_pub_key: [u8; 32],
        rem_url: Option<String>,
        data: Box<dyn bytes::Buf + Send>,
    );
}

type DynTx5PipeClientHandler =
    Arc<dyn Tx5PipeClientHandler + 'static + Send + Sync>;

type Resp = tokio::sync::oneshot::Sender<Tx5PipeResponse>;

#[derive(Default)]
pub(crate) struct RespCache {
    cache: std::collections::HashMap<String, Resp>,
}

impl RespCache {
    fn reg(&mut self, cmd_id: String, r: Resp) {
        self.cache.insert(cmd_id, r);
    }

    fn prune(&mut self) {
        self.cache.retain(|_k, a| !a.is_closed());
    }

    fn resp(&mut self, cmd_id: String, v: Tx5PipeResponse) {
        if let Some(r) = self.cache.remove(&cmd_id) {
            let _ = r.send(v);
        }
    }
}

/// A tx5-pipe client.
pub struct Tx5PipeClient {
    client_impl: DynTx5PipeClientImpl,
    resp_cache: Arc<std::sync::Mutex<RespCache>>,
    prune_cache_task: tokio::task::JoinHandle<()>,
    cmd_id_uniq: std::sync::atomic::AtomicU64,
}

impl Drop for Tx5PipeClient {
    fn drop(&mut self) {
        self.prune_cache_task.abort();
    }
}

impl Tx5PipeClient {
    /// Construct a new tx5-pipe client.
    pub async fn new<I, H, F, C>(client_handler: H, c: C) -> Result<Self>
    where
        I: Tx5PipeClientImpl,
        H: Tx5PipeClientHandler,
        F: Future<Output = Result<I>> + 'static + Send,
        C: FnOnce(Tx5PipeClientIngest) -> F,
    {
        let client_handler: DynTx5PipeClientHandler = Arc::new(client_handler);

        let resp_cache = Arc::new(std::sync::Mutex::new(RespCache::default()));

        let client_ingest = Tx5PipeClientIngest {
            client_handler,
            resp_cache: resp_cache.clone(),
        };

        let client_impl = c(client_ingest).await?;
        let client_impl: DynTx5PipeClientImpl = Arc::new(client_impl);

        let prune_cache_task = {
            let resp_cache = resp_cache.clone();
            tokio::task::spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    resp_cache.lock().unwrap().prune();
                }
            })
        };

        Ok(Tx5PipeClient {
            client_impl,
            resp_cache: resp_cache.clone(),
            prune_cache_task,
            cmd_id_uniq: std::sync::atomic::AtomicU64::new(1),
        })
    }

    /// Assuming you have spawned a Tx5Pipe process as a child process,
    /// and have a `std::io::Read` handle for its stdout, and a `std::io::Write`
    /// handle for its stdin, this function will provide a Tx5PipeClient
    /// instance that can communicate with the Tx5Pipe process.
    pub async fn spawn_std_io<
        H: Tx5PipeClientHandler,
        R: std::io::Read,
        W: std::io::Write,
    >(
        _client_handler: H,
        _stdout: R,
        _stdin: W,
    ) -> Result<Self> {
        todo!()
    }

    /// Spawn a Tx5Pipe "server" in-process and return a client for
    /// communicating through it.
    pub async fn spawn_in_process<H: Tx5PipeClientHandler>(
        client_handler: H,
    ) -> Result<Self> {
        Self::new(client_handler, |ingest| async move {
            struct SrvHnd {
                ingest: Tx5PipeClientIngest,
            }

            impl crate::server::Tx5PipeServerHandler for SrvHnd {
                fn fatal(&self, _error: std::io::Error) {
                    todo!()
                }

                fn pipe(&self, response: Tx5PipeResponse) -> bool {
                    self.ingest.handle_response(response);
                    // TODO always true?
                    true
                }
            }

            let srv =
                crate::server::Tx5PipeServer::new(SrvHnd { ingest }).await?;

            struct SrvImpl {
                srv: Arc<crate::server::Tx5PipeServer>,
            }

            impl Tx5PipeClientImpl for SrvImpl {
                fn shutdown(&self, _error: Error) {}

                fn request(&self, req: Tx5PipeRequest) {
                    self.srv.pipe(req);
                }
            }

            Ok(SrvImpl { srv })
        })
        .await
    }

    fn get_cmd_id(&self) -> String {
        let id = self
            .cmd_id_uniq
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        format!("c{id}")
    }

    fn make_request(
        &self,
        req: Tx5PipeRequest,
    ) -> impl Future<Output = Result<Tx5PipeResponse>> + 'static + Send {
        let cmd_id = req.get_cmd_id();
        let (s, r) = tokio::sync::oneshot::channel();
        self.resp_cache.lock().unwrap().reg(cmd_id, s);
        self.client_impl.request(req);
        async move {
            let resp: Tx5PipeResponse =
                tokio::time::timeout(std::time::Duration::from_secs(20), r)
                    .await
                    .map(|r| r.map_err(|_| Error::id("Timeout")))
                    .map_err(|_| Error::id("Timeout"))??;
            match resp {
                Tx5PipeResponse::Error { text, .. } => Err(Error::err(text)),
                _ => Ok(resp),
            }
        }
    }

    /// Register an application hash.
    pub fn app_reg(
        &self,
        app_hash: [u8; 32],
    ) -> impl Future<Output = Result<()>> + 'static + Send {
        let cmd_id = self.get_cmd_id();
        let req = Tx5PipeRequest::AppReg { cmd_id, app_hash };
        let fut = self.make_request(req);
        async move {
            match fut.await {
                Ok(Tx5PipeResponse::AppRegOk { .. }) => Ok(()),
                Err(err) => Err(err),
                _ => Err(Error::id("InvalidAppRegResponse")),
            }
        }
    }

    /// A request to register as addressable with a signal server.
    /// Returns the addressable client_url.
    pub fn sig_reg(
        &self,
        sig_url: String,
    ) -> impl Future<Output = Result<String>> + 'static + Send {
        let cmd_id = self.get_cmd_id();
        let req = Tx5PipeRequest::SigReg { cmd_id, sig_url };
        let fut = self.make_request(req);
        async move {
            match fut.await {
                Ok(Tx5PipeResponse::SigRegOk { cli_url, .. }) => Ok(cli_url),
                Err(err) => Err(err),
                _ => Err(Error::id("InvalidSigRegResponse")),
            }
        }
    }

    /// A request to make this node discoverable on a bootstrap server.
    pub fn boot_reg(
        &self,
        boot_url: String,
        data: Box<dyn bytes::Buf + Send>,
    ) -> impl Future<Output = Result<()>> + 'static + Send {
        let cmd_id = self.get_cmd_id();
        let req = Tx5PipeRequest::BootReg {
            cmd_id,
            boot_url,
            data,
        };
        let fut = self.make_request(req);
        async move {
            match fut.await {
                Ok(Tx5PipeResponse::BootRegOk { .. }) => Ok(()),
                Err(err) => Err(err),
                _ => Err(Error::id("InvalidBootRegResponse")),
            }
        }
    }

    /// A request to send a message to a remote peer.
    pub fn send(
        &self,
        rem_url: String,
        data: Box<dyn bytes::Buf + Send>,
    ) -> impl Future<Output = Result<()>> + 'static + Send {
        let cmd_id = self.get_cmd_id();
        let req = Tx5PipeRequest::Send {
            cmd_id,
            rem_url,
            data,
        };
        let fut = self.make_request(req);
        async move {
            match fut.await {
                Ok(Tx5PipeResponse::SendOk { .. }) => Ok(()),
                Err(err) => Err(err),
                _ => Err(Error::id("InvalidSendResponse")),
            }
        }
    }
}
