use super::*;

/// Let's just bind to everything and anything as well.
pub(crate) struct Shotgun {
    _v4_listeners: Vec<Arc<Socket>>,
    _v6_listeners: Vec<Arc<Socket>>,
    v4_senders: Vec<Arc<Socket>>,
    v6_senders: Vec<Arc<Socket>>,
    port: u16,
    mcast_v4: std::net::Ipv4Addr,
    mcast_v6: std::net::Ipv6Addr,
}

impl Shotgun {
    /// Construct a new shotgun set of udp sockets.
    pub async fn new(
        mcast_recv: RawRecv,
        ucast_recv: RawRecv,
        port: u16,
        mcast_v4: std::net::Ipv4Addr,
        mcast_v6: std::net::Ipv6Addr,
    ) -> Result<Arc<Shotgun>> {
        let mut errors = Vec::new();
        let mut _v4_listeners = Vec::new();
        let mut _v6_listeners = Vec::new();
        let mut v4_senders = Vec::new();
        let mut v6_senders = Vec::new();

        macro_rules! bind_v4 {
            ($addr:expr) => {{
                match Socket::with_v4(
                    $addr,
                    Some(mcast_v4),
                    port,
                    mcast_recv.clone(),
                )
                .await
                {
                    Ok(socket) => _v4_listeners.push(socket),
                    Err(err) => {
                        tracing::debug!(?err);
                        errors.push(err);
                    }
                }
                // don't bind unicast ports to all interfaces
                if !$addr.is_unspecified() {
                    match Socket::with_v4($addr, None, 0, ucast_recv.clone()).await
                    {
                        Ok(socket) => v4_senders.push(socket),
                        Err(err) => {
                            tracing::debug!(?err);
                            errors.push(err);
                        }
                    }
                }
            }};
        }

        macro_rules! bind_v6 {
            ($addr:expr, $idx:expr) => {{
                match Socket::with_v6(
                    $addr,
                    Some((mcast_v6, $idx)),
                    port,
                    mcast_recv.clone(),
                )
                .await
                {
                    Ok(socket) => _v6_listeners.push(socket),
                    Err(err) => {
                        tracing::debug!(?err);
                        errors.push(err);
                    }
                }
                // don't bind unicast ports to all interfaces
                if !$addr.is_unspecified() {
                    match Socket::with_v6($addr, None, 0, ucast_recv.clone()).await
                    {
                        Ok(socket) => v6_senders.push(socket),
                        Err(err) => {
                            tracing::debug!(?err);
                            errors.push(err);
                        }
                    }
                }
            }};
        }

        bind_v4!(std::net::Ipv4Addr::UNSPECIFIED);
        bind_v6!(std::net::Ipv6Addr::UNSPECIFIED, 0);

        if let Ok(iface_list) = if_addrs::get_if_addrs() {
            for iface in iface_list {
                let ip = iface.ip();

                if ip.is_unspecified() {
                    continue;
                }

                if ip.is_loopback() {
                    continue;
                }

                if ip.ext_is_global() {
                    continue;
                }

                let index = iface.index.unwrap_or_default();

                tracing::info!(?ip, %index, "BINDING");

                match ip {
                    std::net::IpAddr::V4(ip) => bind_v4!(ip),
                    std::net::IpAddr::V6(ip) => bind_v6!(ip, index),
                }
            }
        }

        if _v4_listeners.is_empty() && _v6_listeners.is_empty() {
            return Err(Error::str(format!(
                "could not bind listener: {errors:?}",
            )));
        }

        if v4_senders.is_empty() && v6_senders.is_empty() {
            return Err(Error::str(format!(
                "could not bind sender: {errors:?}",
            )));
        }

        Ok(Arc::new(Self {
            _v4_listeners,
            _v6_listeners,
            v4_senders,
            v6_senders,
            port,
            mcast_v4,
            mcast_v6,
        }))
    }

    /// Send a multicast announcement.
    pub fn multicast(
        &self,
        data: Vec<u8>,
    ) -> impl Future<Output = Result<()>> + 'static + Send {
        let v4 = self
            .v4_senders
            .iter()
            .map(|s| s.socket.clone())
            .collect::<Vec<_>>();
        let v6 = self
            .v6_senders
            .iter()
            .map(|s| s.socket.clone())
            .collect::<Vec<_>>();
        let port = self.port;
        let mcast_v4 = self.mcast_v4;
        let mcast_v6 = self.mcast_v6;
        async move {
            let mut errors = Vec::new();
            let mut success = false;

            let addr = std::net::SocketAddrV4::new(mcast_v4, port);

            for s in v4 {
                match s.send_to(&data, addr).await {
                    Ok(_) => success = true,
                    Err(err) => {
                        tracing::debug!(?err);
                        errors.push(err);
                    }
                }
            }

            let addr = std::net::SocketAddrV6::new(mcast_v6, port, 0, 0);

            for s in v6 {
                match s.send_to(&data, addr).await {
                    Ok(_) => success = true,
                    Err(err) => {
                        tracing::debug!(?err);
                        errors.push(err);
                    }
                }
            }

            if !success {
                return Err(Error::str(format!("failed to send: {errors:?}",)));
            }

            Ok(())
        }
    }
}
