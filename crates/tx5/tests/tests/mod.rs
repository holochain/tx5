use std::sync::Arc;
use tx5::{Endpoint, EndpointRecv, PeerUrl};

mod events;
mod multi_sig;
mod reconnect;
mod relay_over_sig;
mod stress;
mod investigate;

async fn sbd() -> sbd_server::SbdServer {
    let config = sbd_server::Config {
        bind: vec!["127.0.0.1:0".to_string(), "[::1]:0".to_string()],
        limit_clients: 100,
        disable_rate_limiting: true,
        ..Default::default()
    };
    sbd_with_config(config).await
}

async fn sbd_with_config(
    mut config: sbd_server::Config,
) -> sbd_server::SbdServer {
    // Always configure the bind addresses for tests
    config.bind = vec!["127.0.0.1:0".to_string(), "[::1]:0".to_string()];
    sbd_server::SbdServer::new(Arc::new(config)).await.unwrap()
}

async fn ep(s: &sbd_server::SbdServer) -> (PeerUrl, Endpoint, EndpointRecv) {
    let config = tx5::Config {
        signal_allow_plain_text: true, // Always allow plain text for tests
        ..Default::default()
    };

    ep_with_config(s, config).await
}

async fn ep_with_config(
    s: &sbd_server::SbdServer,
    mut config: tx5::Config,
) -> (PeerUrl, Endpoint, EndpointRecv) {
    // Always allow plain text for tests
    config.signal_allow_plain_text = true;

    let (ep, recv) = Endpoint::new(Arc::new(config));
    let sig = format!("ws://{}", s.bind_addrs()[0]);
    let peer_url = ep.listen(tx5::SigUrl::parse(sig).unwrap()).await.unwrap();
    (peer_url, ep, recv)
}

async fn receive_next_message_from(
    r: &mut EndpointRecv,
    url: PeerUrl,
) -> Vec<u8> {
    loop {
        let evt = r.recv().await;
        match evt {
            Some(tx5::EndpointEvent::Message { peer_url, message }) => {
                if peer_url != url {
                    panic!("Received message from unexpected peer: {peer_url}");
                }

                return message;
            }
            Some(evt) => {
                tracing::info!("Received unexpected event: {evt:?}");

                continue; // Ignore other events
            }
            None => panic!("Unexpected end of receiver"),
        }
    }
}

fn enable_tracing() {
    let subscriber = tracing_subscriber::FmtSubscriber::builder()
        .with_env_filter(
            tracing_subscriber::filter::EnvFilter::from_default_env(),
        )
        .with_file(true)
        .with_line_number(true)
        .finish();
    let _ = tracing::subscriber::set_global_default(subscriber);
}
