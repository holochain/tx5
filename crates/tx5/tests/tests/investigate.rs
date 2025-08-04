use crate::tests::{enable_tracing, ep, receive_next_message_from, sbd};

#[tokio::test(flavor = "multi_thread")]
async fn investigate() {
    enable_tracing();

    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let sig = sbd().await;

    let (_p1, e1, mut _r1) = ep(&sig).await;

    let (p2, _e2, mut _r2) = ep(&sig).await;

    e1.send(p2.clone(), b"hello".to_vec()).await.ok();

    tokio::time::sleep(std::time::Duration::from_secs(5)).await;

    // Verify that p1 has no connections:
    assert!(e1.get_stats().connection_list.is_empty(), "p1 should have no connections");
}
