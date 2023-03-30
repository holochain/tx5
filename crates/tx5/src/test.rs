use super::*;

fn init_tracing() {
    let subscriber = tracing_subscriber::FmtSubscriber::builder()
        .with_env_filter(
            tracing_subscriber::filter::EnvFilter::from_default_env(),
        )
        .with_file(true)
        .with_line_number(true)
        .finish();
    let _ = tracing::subscriber::set_global_default(subscriber);
}

#[tokio::test(flavor = "multi_thread")]
async fn endpoint_sanity() {
    init_tracing();

    let mut srv_config = tx5_signal_srv::Config::default();
    srv_config.port = 0;
    srv_config.demo = true;

    let (addr, srv_driver) =
        tx5_signal_srv::exec_tx5_signal_srv(srv_config).unwrap();
    tokio::task::spawn(srv_driver);

    let sig_port = addr.port();

    // TODO remove
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;

    let sig_url = Tx5Url::new(format!("ws://localhost:{}", sig_port)).unwrap();
    println!("sig_url: {}", sig_url);

    let (ep1, _ep_rcv1) = Ep::new().await.unwrap();

    let cli_url1 = ep1.listen(sig_url.clone()).await.unwrap();

    println!("cli_url1: {}", cli_url1);

    let (ep2, mut ep_rcv2) = Ep::new().await.unwrap();

    let cli_url2 = ep2.listen(sig_url).await.unwrap();

    println!("cli_url2: {}", cli_url2);

    ep1.send(cli_url2, &b"hello"[..]).await.unwrap();

    match ep_rcv2.recv().await {
        Some(Ok(EpEvt::Connected { .. })) => (),
        oth => panic!("unexpected: {:?}", oth),
    }

    let recv = ep_rcv2.recv().await;

    match recv {
        Some(Ok(EpEvt::Data {
            rem_cli_url, data, ..
        })) => {
            assert_eq!(cli_url1, rem_cli_url);
            assert_eq!(b"hello", data.to_vec().unwrap().as_slice());
        }
        oth => panic!("unexpected {:?}", oth),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn large_messages() {
    init_tracing();

    let mut srv_config = tx5_signal_srv::Config::default();
    srv_config.port = 0;
    srv_config.demo = true;

    let (addr, srv_driver) =
        tx5_signal_srv::exec_tx5_signal_srv(srv_config).unwrap();
    tokio::task::spawn(srv_driver);

    let sig_port = addr.port();

    let sig_url = Tx5Url::new(format!("ws://localhost:{}", sig_port)).unwrap();
    println!("sig_url: {}", sig_url);

    let (ep1, _ep_rcv1) = Ep::new().await.unwrap();

    let cli_url1 = ep1.listen(sig_url.clone()).await.unwrap();

    println!("cli_url1: {}", cli_url1);

    let (ep2, mut ep_rcv2) = Ep::new().await.unwrap();

    let cli_url2 = ep2.listen(sig_url).await.unwrap();

    println!("cli_url2: {}", cli_url2);

    use rand::Rng;
    let mut rng = rand::thread_rng();
    let mut msg_1 = vec![0; 1024 * 1024 * 16];
    rng.fill(&mut msg_1[..]);
    let mut msg_2 = vec![0; 1024 * 58];
    rng.fill(&mut msg_2[..]);

    let f1 = ep1.send(cli_url2.clone(), msg_1.as_slice());
    let f2 = ep1.send(cli_url2, msg_2.as_slice());

    tokio::try_join!(f1, f2).unwrap();

    match ep_rcv2.recv().await {
        Some(Ok(EpEvt::Connected { .. })) => (),
        oth => panic!("unexpected: {:?}", oth),
    }

    for _ in 0..2 {
        let recv = ep_rcv2.recv().await;
        match recv {
            Some(Ok(EpEvt::Data {
                rem_cli_url, data, ..
            })) => {
                assert_eq!(cli_url1, rem_cli_url);
                let msg = data.to_vec().unwrap();
                if msg.len() == msg_1.len() {
                    assert_eq!(msg, msg_1);
                } else if msg.len() == msg_2.len() {
                    assert_eq!(msg, msg_2);
                } else {
                    panic!("unexpected");
                }
            }
            oth => panic!("unexpected {:?}", oth),
        }
    }
}
