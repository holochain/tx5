use std::sync::Arc;

async fn sbd() -> sbd_server::SbdServer {
    let config = sbd_server::Config {
        bind: vec!["127.0.0.1:0".to_string(), "[::1]:0".to_string()],
        limit_clients: 100,
        disable_rate_limiting: true,
        ..Default::default()
    };
    sbd_server::SbdServer::new(Arc::new(config)).await.unwrap()
}

async fn ep(
    s: &sbd_server::SbdServer,
    test_fail_webrtc: bool,
) -> (tx5::PeerUrl, tx5::Endpoint, tx5::EndpointRecv) {
    let config = tx5::Config {
        signal_allow_plain_text: true,
        test_fail_webrtc,
        ..Default::default()
    };
    let (ep, recv) = tx5::Endpoint::new(Arc::new(config));
    let sig = format!("ws://{}", s.bind_addrs()[0]);
    let peer_url = ep.listen(tx5::SigUrl::parse(sig).unwrap()).await.unwrap();
    (peer_url, ep, recv)
}

#[tokio::test(flavor = "multi_thread")]
async fn connected_event_is_consistent() {
    let sig = sbd().await;
    let (_p1, e1, mut r1) = ep(&sig, false).await;
    let (p2, _e2, mut r2) = ep(&sig, false).await;

    e1.send(p2.clone(), b"hello".to_vec()).await.unwrap();

    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        let mut got_connected = false;
        let mut got_message = false;

        while let Some(evt) = r2.recv().await {
            match evt {
                tx5::EndpointEvent::Connected { .. } => got_connected = true,
                tx5::EndpointEvent::Message { .. } => got_message = true,
                _ => (),
            }
            if got_connected && got_message {
                break;
            }
        }

        while let Some(evt) = r1.recv().await {
            match evt {
                tx5::EndpointEvent::Connected { .. } => break,
                _ => (),
            }
        }
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn connected_event_is_consistent_fail_webrtc() {
    let sig = sbd().await;
    let (_p1, e1, mut r1) = ep(&sig, true).await;
    let (p2, e2, mut r2) = ep(&sig, true).await;

    e1.send(p2.clone(), b"hello".to_vec()).await.unwrap();

    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        let mut got_connected = false;
        let mut got_message = false;

        while let Some(evt) = r2.recv().await {
            match evt {
                tx5::EndpointEvent::Connected { .. } => got_connected = true,
                tx5::EndpointEvent::Disconnected { .. } => {
                    panic!("unexpected disconnect")
                }
                tx5::EndpointEvent::Message { .. } => got_message = true,
                _ => (),
            }
            if got_connected && got_message {
                break;
            }
        }

        while let Some(evt) = r1.recv().await {
            match evt {
                tx5::EndpointEvent::Connected { .. } => break,
                tx5::EndpointEvent::Disconnected { .. } => {
                    panic!("unexpected disconnect")
                }
                _ => (),
            }
        }
    })
    .await
    .unwrap();

    assert!(!e1.get_stats().connection_list[0].is_webrtc);
    assert!(!e2.get_stats().connection_list[0].is_webrtc);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "TODO - this test doesn't pass (https://github.com/holochain/tx5/issues/130)"]
async fn disconnected_event_is_consistent() {
    let sig = sbd().await;
    let (_p1, e1, mut r1) = ep(&sig, false).await;
    let (p2, _e2, mut r2) = ep(&sig, false).await;

    e1.send(p2.clone(), b"hello".to_vec()).await.unwrap();

    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        while let Some(evt) = r1.recv().await {
            match evt {
                tx5::EndpointEvent::Connected { .. } => break,
                _ => (),
            }
        }

        while let Some(evt) = r2.recv().await {
            match evt {
                tx5::EndpointEvent::Connected { .. } => break,
                _ => (),
            }
        }
    })
    .await
    .unwrap();

    e1.close(&p2);

    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        while let Some(evt) = r1.recv().await {
            match evt {
                tx5::EndpointEvent::Disconnected { .. } => break,
                _ => (),
            }
        }

        while let Some(evt) = r2.recv().await {
            match evt {
                tx5::EndpointEvent::Disconnected { .. } => break,
                _ => (),
            }
        }
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "TODO - this test doesn't pass (https://github.com/holochain/tx5/issues/130)"]
async fn disconnected_event_is_consistent_fail_webrtc() {
    let sig = sbd().await;
    let (_p1, e1, mut r1) = ep(&sig, true).await;
    let (p2, _e2, mut r2) = ep(&sig, true).await;

    e1.send(p2.clone(), b"hello".to_vec()).await.unwrap();

    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        while let Some(evt) = r1.recv().await {
            match evt {
                tx5::EndpointEvent::Connected { .. } => break,
                _ => (),
            }
        }

        while let Some(evt) = r2.recv().await {
            match evt {
                tx5::EndpointEvent::Connected { .. } => break,
                _ => (),
            }
        }
    })
    .await
    .unwrap();

    e1.close(&p2);

    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        while let Some(evt) = r1.recv().await {
            match evt {
                tx5::EndpointEvent::Disconnected { .. } => break,
                _ => (),
            }
        }

        while let Some(evt) = r2.recv().await {
            match evt {
                tx5::EndpointEvent::Disconnected { .. } => break,
                _ => (),
            }
        }
    })
    .await
    .unwrap();
}
