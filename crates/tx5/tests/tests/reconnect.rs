use std::time::Duration;

use crate::tests::{
    enable_tracing, ep, ep_with_config, receive_next_message_from, sbd,
};

#[tokio::test(flavor = "multi_thread")]
async fn reconnect_on_webrtc_fail() {
    enable_tracing();

    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let sig = sbd().await;

    let (p1, e1, mut r1) = ep(&sig).await;

    let (p2, e2, mut r2) = ep(&sig).await;

    e1.send(p2.clone(), b"hello".to_vec()).await.unwrap();
    e2.send(p1.clone(), b"world".to_vec()).await.unwrap();

    let msg = receive_next_message_from(&mut r1, p2.clone()).await;
    assert_eq!("world", String::from_utf8_lossy(&msg));

    let msg = receive_next_message_from(&mut r2, p1.clone()).await;
    assert_eq!("hello", String::from_utf8_lossy(&msg));

    assert!(!e1.get_stats().connection_list.is_empty());
    assert!(e1.get_stats().connection_list.iter().all(|c| c.is_webrtc));
    assert!(!e2.get_stats().connection_list.is_empty());
    assert!(e2.get_stats().connection_list.iter().all(|c| c.is_webrtc));

    // Drop the second peer.
    drop(e2);
    drop(r2);

    e1.send(p2, b"still there?".to_vec()).await.unwrap();

    let e1 =
        tokio::time::timeout(std::time::Duration::from_secs(5), async move {
            loop {
                if e1.get_stats().connection_list.is_empty() {
                    break;
                }

                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }

            e1
        })
        .await
        .unwrap();

    // Restart the second endpoint.
    let (p2, _e2, _r2) = ep(&sig).await;

    // Send a message to the new endpoint.
    e1.send(p2.clone(), b"hello again".to_vec()).await.unwrap();

    assert!(!e1.get_stats().connection_list.is_empty());
    assert!(e1.get_stats().connection_list.iter().all(|c| c.is_webrtc));
}

#[tokio::test]
async fn connection_dropped_when_receive_flooded() {
    enable_tracing();

    let sig = sbd().await;
    let (p1, e1, mut r1) = ep(&sig).await;
    let (p2, e2, mut r2) = ep(&sig).await;

    // Both endpoints can successfully send
    e1.send(p2.clone(), b"hello".to_vec()).await.unwrap();
    e2.send(p1.clone(), b"world".to_vec()).await.unwrap();

    // Both endpoints can successfully receive
    let msg = receive_next_message_from(&mut r1, p2.clone()).await;
    assert_eq!("world", String::from_utf8_lossy(&msg));

    let msg = receive_next_message_from(&mut r2, p1.clone()).await;
    assert_eq!("hello", String::from_utf8_lossy(&msg));

    // Flood the receiver with messages until the connection is dropped
    tokio::time::timeout(Duration::from_secs(10), async {
        while !e2.get_stats().connection_list.is_empty() {
            e1.send(p2.clone(), b"hello".to_vec()).await.unwrap();
        }
    })
    .await
    .unwrap();

    // The other side should also drop the connection
    assert!(e1.get_stats().connection_list.is_empty());
}

#[tokio::test]
async fn connection_dropped_when_send_flooded() {
    enable_tracing();

    let sig = sbd().await;
    let (p1, e1, mut r1) = ep(&sig).await;
    let (p2, e2, mut r2) = ep(&sig).await;

    // Both endpoints can successfully send
    e1.send(p2.clone(), b"hello".to_vec()).await.unwrap();
    e2.send(p1.clone(), b"world".to_vec()).await.unwrap();

    // Both endpoints can successfully receive
    let msg = receive_next_message_from(&mut r1, p2.clone()).await;
    assert_eq!("world", String::from_utf8_lossy(&msg));

    let msg = receive_next_message_from(&mut r2, p1.clone()).await;
    assert_eq!("hello", String::from_utf8_lossy(&msg));

    // Now pause time and flood the send channel with messages
    tokio::time::pause();
    while e1.send(p2.clone(), b"hello".to_vec()).await.is_ok() {
        tokio::time::advance(Duration::from_millis(1)).await;
    }
    tokio::time::resume();

    // Can no longer send due to being backed-up so the connection should have been dropped on both
    // sides
    assert!(e1.get_stats().connection_list.is_empty());
    assert!(e2.get_stats().connection_list.is_empty());
}

/// Related to reconnection, because if the connection setup fails, then it must be removed in order
/// for the endpoint to be able to reconnect.
#[tokio::test(flavor = "multi_thread")]
async fn close_connections_on_setup_failure() {
    enable_tracing();

    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let sig = sbd().await;

    let mut config = tx5::Config::new();
    // By enabling both of these, the connection setup is forced to fail because we aren't
    // allowing it to use either of its options.
    // In production this is a misconfiguration, but it is a useful situation for testing that
    // the resources that get allocated on the way to discovering the misconfiguration are
    // cleaned up.
    config.danger_deny_signal_relay = true;
    config.danger_force_signal_relay = true;
    config.timeout = std::time::Duration::from_secs(3);

    let (p1, e1, _r1) = ep_with_config(&sig, config.clone()).await;

    let (p2, e2, _r2) = ep_with_config(&sig, config).await;

    // Each endpoint attempts to send a message to the other.
    //
    // We don't expect the messages to be received, but we do expect the endpoints to clean up
    // any resources that were allocated during the connection setup.
    e1.send(p2.clone(), b"hello".to_vec()).await.ok();
    e2.send(p1.clone(), b"world".to_vec()).await.ok();

    tokio::time::timeout(std::time::Duration::from_secs(10), async move {
        loop {
            // Wait for the endpoints to clean up their resources.
            if e1.get_stats().connection_list.is_empty() && e2.get_stats().connection_list.is_empty() {
                break;
            } else {
                tracing::debug!(
                    "Waiting for endpoints to clean up resources: e1 connections: {}, e2 connections: {}",
                    e1.get_stats().connection_list.len(),
                    e2.get_stats().connection_list.len()
                );
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }).await.unwrap();
}
