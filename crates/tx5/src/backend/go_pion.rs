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

struct GoConRecv(tx5_connection::FramedConnRecv);

impl BackConRecv for GoConRecv {
    fn recv(&mut self) -> BoxFuture<'_, Option<Vec<u8>>> {
        Box::pin(async { self.0.recv().await })
    }
}

struct GoWaitCon(
    PubKey,
    Option<Arc<tx5_connection::Conn>>,
    Option<tx5_connection::ConnRecv>,
);

impl BackWaitCon for GoWaitCon {
    fn wait(
        &mut self,
        recv_limit: Arc<tokio::sync::Semaphore>,
    ) -> BoxFuture<'static, Result<(DynBackCon, DynBackConRecv)>> {
        let con = self.1.take();
        let con_recv = self.2.take();
        Box::pin(async move {
            let (con, con_recv) = match (con, con_recv) {
                (_, None) | (None, _) => {
                    return Err(std::io::Error::other("already awaited"))
                }
                (Some(con), Some(con_recv)) => (con, con_recv),
            };

            con.ready().await;

            let (con, con_recv) =
                tx5_connection::FramedConn::new(con, con_recv, recv_limit)
                    .await?;

            let con: DynBackCon = Arc::new(GoCon(con));
            let con_recv: DynBackConRecv = Box::new(GoConRecv(con_recv));

            Ok((con, con_recv))
        })
    }

    fn pub_key(&self) -> &PubKey {
        &self.0
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
            let wc: DynBackWaitCon =
                Box::new(GoWaitCon(pub_key, Some(con), Some(con_recv)));
            Ok(wc)
        })
    }

    fn pub_key(&self) -> &PubKey {
        self.0.pub_key()
    }
}

struct GoEpRecv(tx5_connection::HubRecv);

impl BackEpRecv for GoEpRecv {
    fn recv(&mut self) -> BoxFuture<'_, Option<DynBackWaitCon>> {
        Box::pin(async {
            let (con, con_recv) = self.0.accept().await?;
            let pub_key = con.pub_key().clone();
            let wc: DynBackWaitCon =
                Box::new(GoWaitCon(pub_key, Some(con), Some(con_recv)));
            Some(wc)
        })
    }
}

/// Get a default version of the module-specific config.
pub fn default_config() -> serde_json::Value {
    serde_json::json!({})
}

/// Connect a new backend based on the tx5-go-pion backend.
pub async fn connect(
    config: &Arc<Config>,
    url: &str,
    listener: bool,
) -> Result<(DynBackEp, DynBackEpRecv)> {
    let webrtc_config = config.initial_webrtc_config.clone().into_bytes();
    let sig_config = tx5_connection::tx5_signal::SignalConfig {
        listener,
        allow_plain_text: config.signal_allow_plain_text,
        //max_connections: config.connection_count_max as usize,
        max_idle: config.timeout,
        ..Default::default()
    };
    let (hub, hub_recv) =
        tx5_connection::Hub::new(webrtc_config, url, Arc::new(sig_config))
            .await?;
    let ep: DynBackEp = Arc::new(GoEp(hub));
    let ep_recv: DynBackEpRecv = Box::new(GoEpRecv(hub_recv));
    Ok((ep, ep_recv))
}
