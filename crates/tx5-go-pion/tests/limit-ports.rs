#[tokio::test(flavor = "multi_thread")]
async fn limit_ports() {
    tx5_core::Tx5InitConfig {
        ephemeral_udp_port_min: 40000,
        ephemeral_udp_port_max: 40000,
    }
    .set_as_global_default()
    .unwrap();

    let (s, mut r) = tokio::sync::mpsc::unbounded_channel();

    let mut con = tx5_go_pion::PeerConnection::new(
        tx5_go_pion::PeerConnectionConfig::default(),
        move |evt| {
            let _ = s.send(evt);
        },
    )
    .await
    .unwrap();

    let _dc = con
        .create_data_channel(tx5_go_pion::DataChannelConfig::default())
        .await
        .unwrap();
    let offer = con
        .create_offer(tx5_go_pion::OfferConfig::default())
        .await
        .unwrap();
    con.set_local_description(offer).await.unwrap();

    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while let Some(evt) = r.recv().await {
            if let tx5_go_pion::PeerConnectionEvent::ICECandidate(mut ice) = evt
            {
                let ice = ice.to_vec().unwrap();
                let ice = String::from_utf8_lossy(&ice);
                let ice = ice.split(' ').collect::<Vec<_>>();
                assert_eq!("40000", ice[5]);
                break;
            }
        }
    })
    .await
    .unwrap();
}
