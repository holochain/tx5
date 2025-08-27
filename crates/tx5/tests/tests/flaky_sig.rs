use crate::tests::{enable_tracing, receive_next_message_from, sbd};
use futures::executor::block_on;
use rand::prelude::SliceRandom;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex;
use tokio::task::AbortHandle;
use tx5::{Endpoint, EndpointRecv, PeerUrl};

#[tokio::test(flavor = "multi_thread")]
async fn reconnect_on_all_signal_ws_disconnect() {
    enable_tracing();

    let sig = sbd().await;

    let flaky_relay = FlakyRelay::connect(sig.bind_addrs()[0].clone()).await;

    let (p1, e1, mut r1) = ep_for_relay(flaky_relay.bound_addr.clone()).await;
    let (p2, e2, mut r2) = ep_for_relay(flaky_relay.bound_addr.clone()).await;

    e1.send(p2.clone(), b"hello".to_vec()).await.unwrap();
    let msg = receive_next_message_from(&mut r2, p1.clone()).await;
    assert_eq!("hello", String::from_utf8_lossy(&msg));

    e2.send(p1.clone(), b"world".to_vec()).await.unwrap();
    let msg = receive_next_message_from(&mut r1, p2.clone()).await;
    assert_eq!("world", String::from_utf8_lossy(&msg));

    // Drop the connections from our endpoints to the signal server via the relay.
    flaky_relay.just_lose_it().await;

    // Now close the direct connections because those would survive the loss of the relay.
    e1.close(&p2);
    e2.close(&p1);

    wait_for_reconnected(&e1, &e2, &mut r1, &mut r2).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn reconnect_on_one_side_signal_ws_disconnect() {
    enable_tracing();

    let sig = sbd().await;

    let flaky_relay = FlakyRelay::connect(sig.bind_addrs()[0].clone()).await;

    let (p1, e1, mut r1) = ep_for_relay(flaky_relay.bound_addr.clone()).await;
    let (p2, e2, mut r2) = ep_for_relay(flaky_relay.bound_addr.clone()).await;

    e1.send(p2.clone(), b"hello".to_vec()).await.unwrap();
    let msg = receive_next_message_from(&mut r2, p1.clone()).await;
    assert_eq!("hello", String::from_utf8_lossy(&msg));

    e2.send(p1.clone(), b"world".to_vec()).await.unwrap();
    let msg = receive_next_message_from(&mut r1, p2.clone()).await;
    assert_eq!("world", String::from_utf8_lossy(&msg));

    // Drop just one of the connections to the signal server via the relay. That forces one side to
    // reconnect while the other side still thinks the connection is alive.
    flaky_relay.just_lose_one().await;

    // Now close the direct connection in one direction.
    e1.close(&p2);

    wait_for_reconnected(&e1, &e2, &mut r1, &mut r2).await;
}

struct FlakyRelay {
    listen_task: AbortHandle,
    inner: Arc<Mutex<Vec<AbortHandle>>>,
    bound_addr: std::net::SocketAddr,
}

impl Drop for FlakyRelay {
    fn drop(&mut self) {
        tracing::debug!("Dropping FlakyRelay");
        self.listen_task.abort();
        block_on(async {
            let mut inner = self.inner.lock().await;
            for task in inner.drain(..) {
                task.abort();
            }
        });
    }
}

impl FlakyRelay {
    async fn connect(target: std::net::SocketAddr) -> Self {
        tracing::debug!(?target, "Connecting to flaky relay");

        let inner = Arc::new(Mutex::new(Vec::new()));
        let (addr_tx, addr_rx) = tokio::sync::oneshot::channel();

        let listen_task = tokio::task::spawn( {
            let inner = inner.clone();
            async move {
                let listener = tokio::net::TcpListener::bind("localhost:0")
                    .await
                    .expect("Failed to bind to local address");

                addr_tx.send(listener.local_addr().unwrap()).unwrap();

                while let Ok((mut socket, _)) = listener.accept().await {
                    tracing::debug!("Accepted connection from {:?}", socket.peer_addr());

                    let mut downstream = tokio::net::TcpSocket::new_v4().unwrap().connect(target).await.unwrap();

                    let relay_task = tokio::task::spawn(async move {
                        loop {
                            let mut rx_buf = vec![0; 1024];
                            let mut tx_buf = vec![0; 1024];

                            tokio::select! {
                                res = socket.read(&mut rx_buf) => {
                                    match res {
                                        Ok(amt) => {
                                            if amt == 0 {
                                                tracing::debug!("Connection closed by peer");
                                                break;
                                            }
                                            tracing::debug!("Received {} bytes from peer", amt);
                                            if let Err(e) = downstream.write(&rx_buf[..amt]).await {
                                                tracing::error!("Failed to write to downstream: {}", e);
                                                break;
                                            }
                                        }
                                        Err(e) => {
                                            tracing::error!("Failed to read from peer: {}", e);
                                            break;
                                        }
                                    }
                                }
                                res = downstream.read(&mut tx_buf) => {
                                    match res {
                                        Ok(amt) => {
                                            if amt == 0 {
                                                tracing::debug!("Downstream connection closed");
                                                break;
                                            }
                                            tracing::debug!("Received {} bytes from downstream", amt);
                                            if let Err(e) = socket.write(&tx_buf[..amt]).await {
                                                tracing::error!("Failed to write to peer: {}", e);
                                                break;
                                            }
                                        }
                                        Err(e) => {
                                            tracing::error!("Failed to read from downstream: {}", e);
                                            break;
                                        }
                                    }
                                }
                            }
                        }

                        tracing::info!("Connection closed");
                    }).abort_handle();

                    inner.lock().await.push(relay_task);
                    tracing::debug!("Relay task spawned for connection");
                }
            }
        }).abort_handle();

        let bound_addr = addr_rx.await.unwrap();

        Self {
            listen_task,
            inner,
            bound_addr,
        }
    }

    /// Drop all connections to the signal server, simulating a flaky relay that
    /// loses all connections.
    async fn just_lose_it(&self) {
        tracing::debug!(
            "Losing connections and fulfilling my purpose as a flaky relay"
        );
        let mut inner = self.inner.lock().await;
        for task in inner.drain(..) {
            tracing::debug!("Aborting a connection task");
            task.abort();
        }
    }

    /// Pick a random connection and abort it, simulating a flaky relay that
    /// can lose individual connections.
    async fn just_lose_one(&self) {
        tracing::debug!(
            "Losing one connection and fulfilling my purpose as a flaky relay"
        );
        let mut inner = self.inner.lock().await;
        inner.shuffle(&mut rand::rng());
        if let Some(task) = inner.pop() {
            task.abort();
            tracing::debug!("One connection lost");
        } else {
            tracing::warn!("No connection to lose");
        }
    }
}

async fn ep_for_relay(
    bound_addr: std::net::SocketAddr,
) -> (PeerUrl, Endpoint, EndpointRecv) {
    let config = tx5::Config::new()
        .with_signal_allow_plain_text(true)
        .with_timeout(std::time::Duration::from_secs(5));

    let (ep, recv) = Endpoint::new(Arc::new(config));
    let sig = format!("ws://{}", bound_addr);
    let peer_url = ep.listen(tx5::SigUrl::parse(sig).unwrap()).await.unwrap();
    (peer_url, ep, recv)
}

/// Expect both peers to be able to reconnect and send messages again. They might have
/// some failures initially, but they should eventually succeed.
async fn wait_for_reconnected(
    e1: &Endpoint,
    e2: &Endpoint,
    r1: &mut EndpointRecv,
    r2: &mut EndpointRecv,
) {
    let mut tries = 0;
    loop {
        tries += 1;

        if tries > 100 {
            panic!("Failed to reconnect after 100 tries");
        }

        let Some(p1) = e1.get_listening_addresses().first().cloned() else {
            tracing::warn!("e1 has no listening addresses, retrying...");
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            continue;
        };
        let Some(p2) = e2.get_listening_addresses().first().cloned() else {
            tracing::warn!("e2 has no listening addresses, retrying...");
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            continue;
        };

        if let Err(err) = e1.send(p2.clone(), b"hello again".to_vec()).await {
            tracing::warn!(?err, "Failed to send message from e1 to e2");
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            continue;
        }

        if let Err(err) = e2.send(p1.clone(), b"world again".to_vec()).await {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            tracing::warn!(?err, "Failed to send message from e2 to e1");
            continue;
        }

        let mut ok_count = 0;
        match tokio::time::timeout(
            std::time::Duration::from_secs(5),
            receive_next_message_from(r1, p2.clone()),
        )
        .await
        {
            Ok(msg) => {
                assert_eq!("world again", String::from_utf8_lossy(&msg));
                ok_count += 1;
            }
            Err(_) => {
                tracing::warn!("Failed to receive message from e2 to e1 after {tries} tries");
                continue;
            }
        }

        match tokio::time::timeout(
            std::time::Duration::from_secs(5),
            receive_next_message_from(r2, p1.clone()),
        )
        .await
        {
            Ok(msg) => {
                assert_eq!("hello again", String::from_utf8_lossy(&msg));
                ok_count += 1;
            }
            Err(_) => {
                tracing::warn!("Failed to receive message from e1 to e2 after {tries} tries");
                continue;
            }
        }

        if ok_count == 2 {
            tracing::info!("Successfully reconnected after {tries} tries");
            break;
        }
    }
}
