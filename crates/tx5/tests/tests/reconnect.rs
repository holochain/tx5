use crate::tests::{enable_tracing, ep, receive_next_message_from, sbd};

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
