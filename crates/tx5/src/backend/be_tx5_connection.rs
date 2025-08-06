//! go pion backend

use super::*;
use crate::Config;

struct GoCon(tx5_connection::FramedConn);

impl BackCon for GoCon {
    fn send(&self, data: Vec<u8>) -> BoxFuture<'_, Result<()>> {
        Box::pin(async { self.0.send(data).await })
    }

    fn pub_key(&self) -> &PubKey {
        self.0.pub_key()
    }

    fn is_using_webrtc(&self) -> bool {
        self.0.is_using_webrtc()
    }

    fn get_stats(&self) -> tx5_connection::ConnStats {
        self.0.get_stats()
    }
}

struct GoConRecvData(tx5_connection::FramedConnRecv);

impl BackConRecvData for GoConRecvData {
    fn recv(&mut self) -> BoxFuture<'_, Option<Vec<u8>>> {
        Box::pin(async { self.0.recv().await })
    }
}

struct GoWaitCon {
    pub_key: PubKey,
    con: Option<Arc<tx5_connection::Conn>>,
    con_recv: Option<tx5_connection::ConnRecv>,
}

impl BackWaitCon for GoWaitCon {
    fn wait(
        &mut self,
        timeout: std::time::Duration,
        recv_limit: Arc<tokio::sync::Semaphore>,
    ) -> BoxFuture<'static, Result<(DynBackCon, DynBackConRecvData)>> {
        let con = self.con.take();
        let con_recv = self.con_recv.take();
        Box::pin(async move {
            let (con, con_recv) = match (con, con_recv) {
                (Some(con), Some(con_recv)) => (con, con_recv),
                _ => return Err(std::io::Error::other("already awaited")),
            };

            // This connection will only ready on a code path that successfully establishes either
            // a WebRTC or a relayed connection. However, there are many exits from the code paths
            // that attempt to set those up. In those cases, we own a semaphore permit that will
            // never be released and we risk deadlocking here without this timeout.
            tokio::time::timeout(timeout, con.ready())
                .await
                .map_err(|e| {
                    std::io::Error::other(format!(
                        "timed out waiting for connection to ready: {e}"
                    ))
                })?;

            let (con, con_recv) =
                tx5_connection::FramedConn::new(con, con_recv, recv_limit)
                    .await?;

            let con: DynBackCon = Arc::new(GoCon(con));
            let con_recv: DynBackConRecvData =
                Box::new(GoConRecvData(con_recv));

            Ok((con, con_recv))
        })
    }

    fn pub_key(&self) -> &PubKey {
        &self.pub_key
    }
}

struct GoEp(tx5_connection::Hub);

impl BackEp for GoEp {
    fn connect(
        &self,
        pub_key: PubKey,
    ) -> BoxFuture<'_, Result<DynBackWaitCon>> {
        Box::pin(async {
            let (con, con_recv) = self.0.connect(pub_key).await?;
            let pub_key = con.pub_key().clone();
            let wc: DynBackWaitCon = Box::new(GoWaitCon {
                pub_key,
                con: Some(con),
                con_recv: Some(con_recv),
            });
            Ok(wc)
        })
    }

    fn pub_key(&self) -> &PubKey {
        self.0.pub_key()
    }
}

struct GoEpRecvCon(tx5_connection::HubRecv);

impl BackEpRecvCon for GoEpRecvCon {
    fn recv(&mut self) -> BoxFuture<'_, Option<DynBackWaitCon>> {
        Box::pin(async {
            let (con, con_recv) = self.0.accept().await?;
            let pub_key = con.pub_key().clone();
            let wc: DynBackWaitCon = Box::new(GoWaitCon {
                pub_key,
                con: Some(con),
                con_recv: Some(con_recv),
            });
            Some(wc)
        })
    }
}

/// Get a default version of the module-specific config.
pub fn default_config() -> serde_json::Value {
    serde_json::json!({})
}

/// Connect a new backend based on the configured backend module.
pub async fn connect(
    config: &Arc<Config>,
    url: &str,
    listener: bool,
) -> Result<(DynBackEp, DynBackEpRecvCon)> {
    let webrtc_config = config.initial_webrtc_config.clone().into_bytes();
    let sig_config = tx5_connection::tx5_signal::SignalConfig {
        listener,
        allow_plain_text: config.signal_allow_plain_text,
        //max_connections: config.connection_count_max as usize,
        max_idle: config.timeout,
        ..Default::default()
    };

    let backend_module = match config.backend_module {
        #[cfg(feature = "backend-libdatachannel")]
        BackendModule::LibDataChannel => {
            tx5_connection::BackendModule::LibDataChannel
        }
        #[cfg(feature = "backend-go-pion")]
        BackendModule::GoPion => tx5_connection::BackendModule::GoPion,
        oth => {
            return Err(std::io::Error::other(format!(
                "unsupported backend module: {oth:?}"
            )))
        }
    };

    let hub_config = Arc::new(tx5_connection::HubConfig {
        backend_module,
        signal_config: Arc::new(sig_config),
    });
    let (hub, hub_recv) =
        tx5_connection::Hub::new(webrtc_config, url, hub_config).await?;
    let ep: DynBackEp = Arc::new(GoEp(hub));
    let ep_recv: DynBackEpRecvCon = Box::new(GoEpRecvCon(hub_recv));
    Ok((ep, ep_recv))
}
