use crate::tests::{
    enable_tracing, ep_with_config, receive_next_message_from, sbd_with_config,
};
use tokio::time::Instant;

#[tokio::test(flavor = "multi_thread")]
async fn relay_over_sig() {
    enable_tracing();

    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let sig = sbd_with_config(sbd_server::Config {
        disable_rate_limiting: false,
        limit_ip_kbps: 100_000,
        limit_ip_byte_burst: 26000000,
        ..Default::default()
    })
    .await;

    let (p1, e1, mut r1) = ep_with_config(
        &sig,
        tx5::Config {
            danger_force_signal_relay: true,
            ..Default::default()
        },
    )
    .await;

    let (p2, e2, mut r2) = ep_with_config(
        &sig,
        tx5::Config {
            danger_force_signal_relay: true,
            ..Default::default()
        },
    )
    .await;

    // Now we should have two endpoints connected to the same signal server which rely on relaying
    // messages over the signal server.

    e1.send(p2.clone(), b"hello".to_vec()).await.unwrap();
    e2.send(p1.clone(), b"world".to_vec()).await.unwrap();

    let msg = receive_next_message_from(&mut r1, p2.clone()).await;
    assert_eq!("world", String::from_utf8_lossy(&msg));

    let msg = receive_next_message_from(&mut r2, p1.clone()).await;
    assert_eq!("hello", String::from_utf8_lossy(&msg));

    // Now we should be able to send larger messages, up to the maximum size of data we normally
    // expect to be able to send over a WebRTC connection.

    let large_msg = vec![97; 1024 * 500]; // 500 KiB of data

    e1.send(p2.clone(), large_msg.clone()).await.unwrap();
    let msg = receive_next_message_from(&mut r2, p1.clone()).await;
    assert_eq!(large_msg, msg);

    e2.send(p1.clone(), large_msg.clone()).await.unwrap();
    let msg = receive_next_message_from(&mut r1, p2.clone()).await;
    assert_eq!(large_msg, msg);

    // Send a batch of large messages

    let end = Instant::now() + std::time::Duration::from_secs(60);
    for _ in 0..20 {
        e1.send(p2.clone(), large_msg.clone()).await.unwrap();
    }

    // Should be able to receive all of them within the timeout
    tokio::time::timeout_at(end, async {
        for _ in 0..20 {
            let msg = receive_next_message_from(&mut r2, p1.clone()).await;
            assert_eq!(large_msg, msg);
        }
    })
    .await
    .unwrap();
}
