#![deny(missing_docs)]
#![deny(unsafe_code)]
#![deny(warnings)]
#![doc = tx5_core::__doc_header!()]
//! # tx5-pipe
//!
//! Holochain WebRTC P2P CLI Tool Controller.

use bytes::*;
use std::future::Future;
use std::io::Result;
use std::sync::Arc;
use tx5_core::pipe_ipc::*;
use tx5_core::Error;

fn into_vec<B: bytes::Buf>(mut b: B) -> Vec<u8> {
    let mut out = Vec::with_capacity(b.remaining());
    while b.has_remaining() {
        let c = b.chunk();
        out.extend_from_slice(c);
        b.advance(c.len());
    }
    out
}

/// Use these types when writing your own tx5-pipe control impl.
pub mod control_impl {
    use super::*;

    /// Use this to report messages when writing your own tx5-pipe control impl.
    pub struct Tx5PipeControlIngest {
        pub(crate) resp_cache: Arc<std::sync::Mutex<RespCache>>,
        pub(crate) control_handler: DynTx5PipeControlHandler,
    }

    impl Tx5PipeControlIngest {
        /// Report a message from the server.
        pub fn handle_response(&self, resp: Tx5PipeResponse) {
            if let Some(cmd_id) = resp.get_cmd_id() {
                self.resp_cache.lock().unwrap().resp(cmd_id.clone(), resp);
                return;
            }

            match resp {
                Tx5PipeResponse::Error { .. }
                | Tx5PipeResponse::SigRegOk { .. }
                | Tx5PipeResponse::BootRegOk { .. }
                | Tx5PipeResponse::BootQueryOk { .. }
                | Tx5PipeResponse::SendOk { .. }
                | Tx5PipeResponse::BootQueryResp { .. } => unreachable!(),
                Tx5PipeResponse::Help { .. } => (),
                Tx5PipeResponse::Recv { rem_url, data } => {
                    self.control_handler.recv(rem_url, data);
                }
            }
        }
    }

    /// Implement this to write your own tx5-pipe control impl.
    pub trait Tx5PipeControlImpl: 'static + Send + Sync {
        /// Shut down the pipe.
        fn shutdown(&self, error: Error);

        /// Send a request to the server.
        fn request(&self, req: Tx5PipeRequest);
    }

    pub(crate) type DynTx5PipeControlImpl =
        Arc<dyn Tx5PipeControlImpl + 'static + Send + Sync>;
}
pub(crate) use control_impl::*;

/// Implement this to handle unsolicited incoming tx5-pipe messages.
pub trait Tx5PipeControlHandler: 'static + Send + Sync {
    /// Incoming message from remote.
    fn recv(&self, rem_url: String, data: Box<dyn bytes::Buf + Send>);
}

type DynTx5PipeControlHandler =
    Arc<dyn Tx5PipeControlHandler + 'static + Send + Sync>;

type Resp =
    tokio::sync::oneshot::Sender<(Tx5PipeResponse, Vec<Tx5PipeResponse>)>;

#[derive(Default)]
pub(crate) struct RespCache {
    cache: std::collections::HashMap<String, (Resp, Vec<Tx5PipeResponse>)>,
}

impl RespCache {
    fn reg(&mut self, cmd_id: String, r: Resp) {
        self.cache.insert(cmd_id, (r, Vec::new()));
    }

    fn prune(&mut self) {
        self.cache.retain(|_k, (a, _)| !a.is_closed());
    }

    fn resp(&mut self, cmd_id: String, v: Tx5PipeResponse) {
        if matches!(&v, Tx5PipeResponse::BootQueryResp { .. }) {
            if let Some(e) = self.cache.get_mut(&cmd_id) {
                e.1.push(v);
            }
        } else if let Some((r, additional)) = self.cache.remove(&cmd_id) {
            let _ = r.send((v, additional));
        }
    }
}

/// BootQueryResp struct.
#[derive(Debug)]
pub struct BootQueryResp {
    /// Remote pub_key.
    pub rem_pub_key: [u8; 32],

    /// Remote url.
    pub rem_url: Option<String>,

    /// Expiration unix epoch seconds.
    pub expires_at_s: u64,

    /// Bootstrap meta data.
    pub data: Vec<u8>,
}

/// A tx5-pipe control.
pub struct Tx5PipeControl {
    control_impl: DynTx5PipeControlImpl,
    resp_cache: Arc<std::sync::Mutex<RespCache>>,
    prune_cache_task: tokio::task::JoinHandle<()>,
    cmd_id_uniq: std::sync::atomic::AtomicU64,
}

impl Drop for Tx5PipeControl {
    fn drop(&mut self) {
        self.prune_cache_task.abort();
    }
}

impl Tx5PipeControl {
    /// Construct a new tx5-pipe control.
    pub async fn new<I, H, F, C>(control_handler: H, c: C) -> Result<Self>
    where
        I: Tx5PipeControlImpl,
        H: Tx5PipeControlHandler,
        F: Future<Output = Result<I>> + 'static + Send,
        C: FnOnce(Tx5PipeControlIngest) -> F,
    {
        let control_handler: DynTx5PipeControlHandler =
            Arc::new(control_handler);

        let resp_cache = Arc::new(std::sync::Mutex::new(RespCache::default()));

        let control_ingest = Tx5PipeControlIngest {
            control_handler,
            resp_cache: resp_cache.clone(),
        };

        let control_impl = c(control_ingest).await?;
        let control_impl: DynTx5PipeControlImpl = Arc::new(control_impl);

        let prune_cache_task = {
            let resp_cache = resp_cache.clone();
            tokio::task::spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    resp_cache.lock().unwrap().prune();
                }
            })
        };

        Ok(Tx5PipeControl {
            control_impl,
            resp_cache: resp_cache.clone(),
            prune_cache_task,
            cmd_id_uniq: std::sync::atomic::AtomicU64::new(1),
        })
    }

    /// Spawn tx5-pipe as a child process.
    /// Delegates to spawn_async_io.
    pub async fn spawn_child<
        H: Tx5PipeControlHandler,
        P: AsRef<std::ffi::OsStr>,
    >(
        control_handler: H,
        exec_path: P,
    ) -> Result<(Self, tokio::process::Child)> {
        let mut child = tokio::process::Command::new(exec_path)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .kill_on_drop(true)
            .spawn()?;

        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();

        let this = Self::spawn_async_io(control_handler, stdout, stdin).await?;

        Ok((this, child))
    }

    /// Assuming you have spawned a Tx5Pipe process as a child process,
    /// or you have some other equivalent [tokio::io::AsyncRead] handle
    /// to a pipe stdout and [tokio::io::AsyncWrite] handle to a pipe stdin,
    /// this function will provide a Tx5PipeControl instance that can
    /// communicate with the Tx5Pipe server process.
    pub async fn spawn_async_io<
        H: Tx5PipeControlHandler,
        R: tokio::io::AsyncRead + 'static + Send + Unpin,
        W: tokio::io::AsyncWrite + 'static + Send + Unpin,
    >(
        control_handler: H,
        mut stdout: R,
        mut stdin: W,
    ) -> Result<Self> {
        Self::new(control_handler, |ingest| async move {
            tokio::task::spawn(async move {
                use tokio::io::AsyncReadExt;

                const LOW_CAP: usize = 1024;
                const HIGH_CAP: usize = 8 * LOW_CAP;

                let mut parser = asv::AsvParser::default();

                let mut buf = BytesMut::with_capacity(HIGH_CAP);

                loop {
                    if buf.capacity() < LOW_CAP {
                        std::mem::swap(
                            &mut buf,
                            &mut BytesMut::with_capacity(HIGH_CAP),
                        );
                    }

                    stdout.read_buf(&mut buf).await?;

                    for field_list in parser.parse(buf.split_to(buf.len()))? {
                        let res = Tx5PipeResponse::decode(field_list)?;
                        ingest.handle_response(res);
                    }
                }

                #[allow(unreachable_code)]
                Result::Ok(())
            });

            struct SrvImpl(tokio::sync::mpsc::UnboundedSender<Tx5PipeRequest>);

            impl Tx5PipeControlImpl for SrvImpl {
                fn shutdown(&self, _error: Error) {}

                fn request(&self, req: Tx5PipeRequest) {
                    let _ = self.0.send(req);
                }
            }

            let (s, mut r) =
                tokio::sync::mpsc::unbounded_channel::<Tx5PipeRequest>();

            tokio::task::spawn(async move {
                use tokio::io::AsyncWriteExt;

                let mut enc = asv::AsvEncoder::default();

                while let Some(req) = r.recv().await {
                    req.encode(&mut enc)?;

                    while let Ok(req) = r.try_recv() {
                        req.encode(&mut enc)?;
                    }

                    let mut buf = enc.drain();
                    while buf.has_remaining() {
                        let c = buf.chunk();
                        stdin.write_all(c).await?;
                        buf.advance(c.len());
                    }
                }

                Result::Ok(())
            });

            Ok(SrvImpl(s))
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
    ) -> impl Future<Output = Result<(Tx5PipeResponse, Vec<Tx5PipeResponse>)>>
           + 'static
           + Send {
        let cmd_id = req.get_cmd_id();
        let (s, r) = tokio::sync::oneshot::channel();
        self.resp_cache.lock().unwrap().reg(cmd_id, s);
        self.control_impl.request(req);
        async move {
            let resp: (Tx5PipeResponse, Vec<Tx5PipeResponse>) =
                tokio::time::timeout(std::time::Duration::from_secs(20), r)
                    .await
                    .map(|r| r.map_err(|_| Error::id("Timeout")))
                    .map_err(|_| Error::id("Timeout"))??;
            match resp {
                (Tx5PipeResponse::Error { text, .. }, _) => {
                    Err(Error::err(text))
                }
                _ => Ok(resp),
            }
        }
    }

    /// A request to register as addressable with a signal server.
    /// Returns the addressable control_url.
    pub fn sig_reg(
        &self,
        sig_url: String,
    ) -> impl Future<Output = Result<String>> + 'static + Send {
        let cmd_id = self.get_cmd_id();
        let req = Tx5PipeRequest::SigReg { cmd_id, sig_url };
        let fut = self.make_request(req);
        async move {
            match fut.await {
                Ok((Tx5PipeResponse::SigRegOk { cli_url, .. }, _)) => {
                    Ok(cli_url)
                }
                Err(err) => Err(err),
                _ => Err(Error::id("InvalidSigRegResponse")),
            }
        }
    }

    /// A request to make this node discoverable on a bootstrap server.
    pub fn boot_reg(
        &self,
        boot_url: String,
        app_hash: [u8; 32],
        cli_url: Option<String>,
        data: Box<dyn bytes::Buf + Send>,
    ) -> impl Future<Output = Result<()>> + 'static + Send {
        let cmd_id = self.get_cmd_id();
        let req = Tx5PipeRequest::BootReg {
            cmd_id,
            boot_url,
            app_hash,
            cli_url,
            data,
        };
        let fut = self.make_request(req);
        async move {
            match fut.await {
                Ok((Tx5PipeResponse::BootRegOk { .. }, _)) => Ok(()),
                Err(err) => Err(err),
                _ => Err(Error::id("InvalidBootRegResponse")),
            }
        }
    }

    /// Query a bootstrap server for peers on a given app hash.
    pub fn boot_query(
        &self,
        boot_url: String,
        app_hash: [u8; 32],
    ) -> impl Future<Output = Result<Vec<BootQueryResp>>> + 'static + Send {
        let cmd_id = self.get_cmd_id();
        let req = Tx5PipeRequest::BootQuery {
            cmd_id,
            boot_url,
            app_hash,
        };
        let fut = self.make_request(req);
        async move {
            match fut.await {
                Ok((Tx5PipeResponse::BootQueryOk { .. }, additional)) => {
                    Ok(additional
                        .into_iter()
                        .filter_map(|r| match r {
                            Tx5PipeResponse::BootQueryResp {
                                rem_pub_key,
                                rem_url,
                                expires_at_s,
                                data,
                                ..
                            } => Some(BootQueryResp {
                                rem_pub_key,
                                rem_url,
                                expires_at_s,
                                data: into_vec(data),
                            }),
                            _ => None,
                        })
                        .collect())
                }
                Err(err) => Err(err),
                _ => Err(Error::id("InvalidBootQueryResponse")),
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
                Ok((Tx5PipeResponse::SendOk { .. }, _)) => Ok(()),
                Err(err) => Err(err),
                _ => Err(Error::id("InvalidSendResponse")),
            }
        }
    }
}
