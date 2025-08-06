use crate::tests::{enable_tracing, ep, receive_next_message_from, sbd};

#[ignore]
#[tokio::test(flavor = "multi_thread")]
async fn investigate() {
    enable_tracing();

    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let sig = sbd().await;

    let (p1, e1, mut r1) = ep(&sig).await;
    println!("[1] Listening on: {:?}", p1);

    let (p2, e2, mut r2) = ep(&sig).await;
    println!("[2] Listening on: {:?}", p2);

    e1.send(p2.clone(), b"hello".to_vec()).await.ok();
    let got = receive_next_message_from(&mut r2, p1).await;
    tracing::info!("Received message: {:?}", String::from_utf8_lossy(&got));

    tracing::info!("Dropping all connections...");

    sig.drop_all_connections().await;
    tracing::info!("All connections dropped, waiting for 5 seconds...");

    // Close the connection to p2, so that we'll need to reconnect over the missing signal server.
    e1.close(&p2);

    e1.send(p2.clone(), b"world".to_vec()).await.ok();
    match tokio::time::timeout(
        std::time::Duration::from_secs(10),
        receive_next_message_from(&mut r1, p2.clone()),
    )
    .await
    {
        Ok(msg) => {
            tracing::info!(
                "Received message from p2: {:?}",
                String::from_utf8_lossy(&msg)
            );
        }
        Err(_) => {
            tracing::warn!("Timed out waiting for message from p2");
        }
    };

    // Sleep while the connection setup fails. We should be in a clean state with no connections now.
    tokio::time::sleep(std::time::Duration::from_secs(61)).await;

    println!("e1 stats: {:?}", e1.get_stats());

    let p1 = e1.get_listening_addresses()[0].clone();
    println!("[1] Reconnecting to: {:?}", p1);

    let p2 = e2.get_listening_addresses()[0].clone();
    println!("[2] Reconnecting to: {:?}", p2);

    for i in 0..100 {
        tracing::info!("Sending message {} from p1 to p2", i);
        let p2 = e2.get_listening_addresses()[0].clone();
        try_send_receive(
            &e1,
            &mut r2,
            &p2,
            format!("Message {} from p1 to p2", i).as_bytes(),
        )
        .await;
    }
}

async fn try_send_receive(
    sender: &crate::tests::Endpoint,
    receiver: &mut crate::tests::EndpointRecv,
    p: &crate::tests::PeerUrl,
    msg: &[u8],
) {
    sender.send(p.clone(), msg.to_vec()).await.ok();
    match tokio::time::timeout(
        std::time::Duration::from_secs(10),
        receive_next_message_from(receiver, p.clone()),
    )
    .await
    {
        Ok(msg) => {
            tracing::info!(
                "Received message from p2: {:?}",
                String::from_utf8_lossy(&msg)
            );
        }
        Err(_) => {
            tracing::warn!("Timed out waiting for message from p2");
        }
    }
}
