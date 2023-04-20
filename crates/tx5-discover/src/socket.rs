use super::*;

/// Raw udp socket wrapper.
pub(crate) struct Socket {
    pub(crate) socket: Arc<tokio::net::UdpSocket>,
    recv_task: tokio::task::JoinHandle<()>,
}

impl Drop for Socket {
    fn drop(&mut self) {
        self.recv_task.abort();
    }
}

impl Socket {
    /// Bind a new ipv4 udp socket.
    pub async fn with_v4(
        iface: std::net::Ipv4Addr,
        mcast: Option<std::net::Ipv4Addr>,
        port: u16,
        raw_recv: RawRecv,
    ) -> Result<Arc<Self>> {
        let s = socket2::Socket::new(
            socket2::Domain::IPV4,
            socket2::Type::DGRAM,
            Some(socket2::Protocol::UDP),
        )?;

        s.set_nonblocking(true)?;

        if let Some(mcast) = mcast {
            s.set_reuse_address(true)?;
            #[cfg(unix)]
            s.set_reuse_port(true)?;
            s.join_multicast_v4(&mcast, &std::net::Ipv4Addr::UNSPECIFIED)?;
            s.set_multicast_loop_v4(true)?;
            s.set_ttl(1)?;
        }

        let bind_addr = std::net::SocketAddrV4::new(iface, port);
        s.bind(&bind_addr.into())?;

        let socket = Arc::new(tokio::net::UdpSocket::from_std(s.into())?);

        let recv_sock = Arc::downgrade(&socket);
        let recv_task = tokio::task::spawn(async move {
            let mut buf = [0; MAX_PACKET_SIZE];
            loop {
                let socket = match recv_sock.upgrade() {
                    None => break,
                    Some(recv_sock) => recv_sock,
                };

                match socket.recv_from(&mut buf).await {
                    Ok((len, addr)) => {
                        let data = buf[..len].to_vec();
                        raw_recv(Ok((recv_sock.clone(), data, addr)));
                    }
                    Err(err) => raw_recv(Err(err)),
                }
            }
        });

        Ok(Arc::new(Self { socket, recv_task }))
    }

    /// Bind a new ipv6 udp socket.
    pub async fn with_v6(
        iface: std::net::Ipv6Addr,
        mcast: Option<(std::net::Ipv6Addr, u32)>,
        port: u16,
        raw_recv: RawRecv,
    ) -> Result<Arc<Self>> {
        let s = socket2::Socket::new(
            socket2::Domain::IPV6,
            socket2::Type::DGRAM,
            Some(socket2::Protocol::UDP),
        )?;

        s.set_nonblocking(true)?;

        if let Some((mcast, index)) = mcast {
            s.set_reuse_address(true)?;
            #[cfg(unix)]
            s.set_reuse_port(true)?;
            s.set_only_v6(true)?;
            assert!(mcast.is_multicast());
            s.join_multicast_v6(&mcast, index)?;
            s.set_multicast_loop_v6(true)?;
            s.set_multicast_hops_v6(1)?;
        }

        let bind_addr = std::net::SocketAddrV6::new(iface, port, 0, 0);
        s.bind(&bind_addr.into())?;

        let socket = Arc::new(tokio::net::UdpSocket::from_std(s.into())?);

        let recv_sock = Arc::downgrade(&socket);
        let recv_task = tokio::task::spawn(async move {
            let mut buf = [0; 4096];
            loop {
                let socket = match recv_sock.upgrade() {
                    None => break,
                    Some(recv_sock) => recv_sock,
                };

                match socket.recv_from(&mut buf).await {
                    Ok((len, addr)) => {
                        let data = buf[..len].to_vec();
                        raw_recv(Ok((recv_sock.clone(), data, addr)));
                    }
                    Err(err) => raw_recv(Err(err)),
                }
            }
        });

        Ok(Arc::new(Self { socket, recv_task }))
    }
}
