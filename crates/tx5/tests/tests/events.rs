use crate::tests::{ep, ep_with_config, sbd};

#[tokio::test(flavor = "multi_thread")]
async fn connected_event_is_consistent() {
    let sig = sbd().await;
    let (_p1, e1, mut r1) = ep(&sig).await;
    let (p2, _e2, mut r2) = ep(&sig).await;

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

    let mut config = tx5::Config::new();
    config.danger_force_signal_relay = true;

    let (_p1, e1, mut r1) = ep_with_config(&sig, config.clone()).await;
    let (p2, e2, mut r2) = ep_with_config(&sig, config).await;

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
    let (_p1, e1, mut r1) = ep(&sig).await;
    let (p2, _e2, mut r2) = ep(&sig).await;

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

    let mut config = tx5::Config::new();
    config.danger_force_signal_relay = true;

    let (_p1, e1, mut r1) = ep_with_config(&sig, config.clone()).await;
    let (p2, _e2, mut r2) = ep_with_config(&sig, config).await;

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
