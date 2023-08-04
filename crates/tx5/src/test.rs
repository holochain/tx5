use super::*;
use std::sync::Arc;

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

    let tmp = tempfile::tempdir().unwrap();

    let (influxive, meter_provider) =
        influxive::influxive_child_process_meter_provider(
            influxive::InfluxiveChildSvcConfig::default()
                .with_database_path(Some(tmp.path().to_owned())),
            influxive::InfluxiveMeterProviderConfig::default()
                .with_observable_report_interval(Some(
                    std::time::Duration::from_millis(200),
                )),
        )
        .await
        .unwrap();
    opentelemetry_api::global::set_meter_provider(meter_provider);

    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    let mut srv_config = tx5_signal_srv::Config::default();
    srv_config.port = 0;
    srv_config.demo = true;

    let (srv_driver, addr_list, _) =
        tx5_signal_srv::exec_tx5_signal_srv(srv_config).unwrap();
    tokio::task::spawn(srv_driver);

    let sig_port = addr_list.get(0).unwrap().port();

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

    ep1.ban([42; 32].into(), std::time::Duration::from_secs(42));
    ep2.ban([43; 32].into(), std::time::Duration::from_secs(43));

    println!(
        "{}",
        serde_json::to_string_pretty(&ep1.get_stats().await.unwrap()).unwrap()
    );
    println!(
        "{}",
        serde_json::to_string_pretty(&ep2.get_stats().await.unwrap()).unwrap()
    );

    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    let metrics = influxive
        .query(
            r#"from(bucket: "influxive")
|> range(start: -15m, stop: now())
"#,
        )
        .await
        .unwrap();

    println!("@@@@@@\n{metrics}\n@@@@@@");

    assert!(metrics.matches("tx5.endpoint.conn.count").count() > 0);
    assert!(metrics.matches("tx5.endpoint.conn.recv.By").count() > 0);
    assert!(metrics.matches("tx5.endpoint.conn.send.By").count() > 0);

    influxive.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn disconnect() {
    init_tracing();

    let mut srv_config = tx5_signal_srv::Config::default();
    srv_config.port = 0;
    srv_config.demo = true;

    let (srv_driver, addr_list, _) =
        tx5_signal_srv::exec_tx5_signal_srv(srv_config).unwrap();
    tokio::task::spawn(srv_driver);

    let sig_port = addr_list.get(0).unwrap().port();

    let sig_url = Tx5Url::new(format!("ws://localhost:{}", sig_port)).unwrap();
    println!("sig_url: {}", sig_url);

    let conf = DefConfig::default()
        .with_max_conn_init(std::time::Duration::from_secs(8));

    let (ep1, mut ep_rcv1) = Ep::with_config(conf).await.unwrap();

    let cli_url1 = ep1.listen(sig_url.clone()).await.unwrap();

    println!("cli_url1: {}", cli_url1);

    let (ep2, mut ep_rcv2) = Ep::new().await.unwrap();

    let cli_url2 = ep2.listen(sig_url).await.unwrap();

    println!("cli_url2: {}", cli_url2);

    ep1.send(cli_url2.clone(), &b"hello"[..]).await.unwrap();

    match ep_rcv1.recv().await {
        Some(Ok(EpEvt::Connected { .. })) => (),
        oth => panic!("unexpected: {:?}", oth),
    }

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

    ep2.ban(cli_url1.id().unwrap(), std::time::Duration::from_secs(43));

    match ep_rcv1.recv().await {
        Some(Ok(EpEvt::Disconnected { .. })) => (),
        oth => panic!("unexpected: {:?}", oth),
    }

    assert!(ep1.send(cli_url2, &b"hello"[..]).await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn connect_timeout() {
    init_tracing();

    let mut srv_config = tx5_signal_srv::Config::default();
    srv_config.port = 0;
    srv_config.demo = true;

    let (srv_driver, addr_list, _) =
        tx5_signal_srv::exec_tx5_signal_srv(srv_config).unwrap();
    tokio::task::spawn(srv_driver);

    let sig_port = addr_list.get(0).unwrap().port();

    let sig_url = Tx5Url::new(format!("ws://localhost:{}", sig_port)).unwrap();
    println!("sig_url: {}", sig_url);

    let conf = DefConfig::default()
        .with_max_conn_init(std::time::Duration::from_secs(8));

    let (ep1, _ep_rcv1) = Ep::with_config(conf).await.unwrap();

    ep1.listen(sig_url.clone()).await.unwrap();

    let cli_url_fake = sig_url.to_client([0xdb; 32].into());

    let start = std::time::Instant::now();

    assert!(ep1.send(cli_url_fake, &b"hello"[..]).await.is_err());

    assert!(
        start.elapsed().as_secs_f64() < 10.0,
        "expected timeout in 8 seconds, timed out in {} seconds",
        start.elapsed().as_secs_f64()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn preflight_small() {
    init_tracing();

    let mut srv_config = tx5_signal_srv::Config::default();
    srv_config.port = 0;
    srv_config.demo = true;

    let (srv_driver, addr_list, _) =
        tx5_signal_srv::exec_tx5_signal_srv(srv_config).unwrap();
    tokio::task::spawn(srv_driver);

    let sig_port = addr_list.get(0).unwrap().port();

    let sig_url = Tx5Url::new(format!("ws://localhost:{}", sig_port)).unwrap();
    println!("sig_url: {}", sig_url);

    let valid_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    const SMALL_DATA: &[u8] = &[42; 16];

    fn make_config(
        valid_count: Arc<std::sync::atomic::AtomicUsize>,
    ) -> DefConfig {
        DefConfig::default()
            .with_conn_preflight(|_, _| {
                println!("PREFLIGHT");
                Box::pin(async move {
                    Ok(Some(bytes::Bytes::from_static(SMALL_DATA)))
                })
            })
            .with_conn_validate(move |_, _, data| {
                println!("VALIDATE");
                assert_eq!(SMALL_DATA, data.unwrap());
                valid_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Box::pin(async move { Ok(()) })
            })
    }

    let (ep1, mut ep_rcv1) = Ep::with_config(make_config(valid_count.clone()))
        .await
        .unwrap();
    let cli_url1 = ep1.listen(sig_url.clone()).await.unwrap();
    println!("cli_url1: {}", cli_url1);

    let (ep2, mut ep_rcv2) = Ep::with_config(make_config(valid_count.clone()))
        .await
        .unwrap();
    let cli_url2 = ep2.listen(sig_url).await.unwrap();
    println!("cli_url2: {}", cli_url2);

    ep1.send(cli_url2, &b"hello"[..]).await.unwrap();

    match ep_rcv1.recv().await {
        Some(Ok(EpEvt::Connected { .. })) => (),
        oth => panic!("unexpected: {:?}", oth),
    }

    match ep_rcv2.recv().await {
        Some(Ok(EpEvt::Connected { .. })) => (),
        oth => panic!("unexpected: {:?}", oth),
    }

    assert_eq!(2, valid_count.load(std::sync::atomic::Ordering::SeqCst));
}

#[tokio::test(flavor = "multi_thread")]
async fn preflight_huge() {
    init_tracing();

    let mut srv_config = tx5_signal_srv::Config::default();
    srv_config.port = 0;
    srv_config.demo = true;

    let (srv_driver, addr_list, _) =
        tx5_signal_srv::exec_tx5_signal_srv(srv_config).unwrap();
    tokio::task::spawn(srv_driver);

    let sig_port = addr_list.get(0).unwrap().port();

    let sig_url = Tx5Url::new(format!("ws://localhost:{}", sig_port)).unwrap();
    println!("sig_url: {}", sig_url);

    let valid_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    const HUGE_DATA: &[u8] = &[42; 16 * 1024 * 512];

    fn make_config(
        valid_count: Arc<std::sync::atomic::AtomicUsize>,
    ) -> DefConfig {
        DefConfig::default()
            .with_conn_preflight(|_, _| {
                println!("PREFLIGHT");
                Box::pin(async move {
                    Ok(Some(bytes::Bytes::from_static(HUGE_DATA)))
                })
            })
            .with_conn_validate(move |_, _, data| {
                println!("VALIDATE");
                assert_eq!(HUGE_DATA, data.unwrap());
                valid_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Box::pin(async move { Ok(()) })
            })
    }

    let (ep1, mut ep_rcv1) = Ep::with_config(make_config(valid_count.clone()))
        .await
        .unwrap();
    let cli_url1 = ep1.listen(sig_url.clone()).await.unwrap();
    println!("cli_url1: {}", cli_url1);

    let (ep2, mut ep_rcv2) = Ep::with_config(make_config(valid_count.clone()))
        .await
        .unwrap();
    let cli_url2 = ep2.listen(sig_url).await.unwrap();
    println!("cli_url2: {}", cli_url2);

    ep1.send(cli_url2, &b"hello"[..]).await.unwrap();

    match ep_rcv1.recv().await {
        Some(Ok(EpEvt::Connected { .. })) => (),
        oth => panic!("unexpected: {:?}", oth),
    }

    match ep_rcv2.recv().await {
        Some(Ok(EpEvt::Connected { .. })) => (),
        oth => panic!("unexpected: {:?}", oth),
    }

    assert_eq!(2, valid_count.load(std::sync::atomic::Ordering::SeqCst));
}

#[tokio::test(flavor = "multi_thread")]
async fn ban() {
    init_tracing();

    let mut srv_config = tx5_signal_srv::Config::default();
    srv_config.port = 0;
    srv_config.demo = true;

    let (srv_driver, addr_list, _) =
        tx5_signal_srv::exec_tx5_signal_srv(srv_config).unwrap();
    tokio::task::spawn(srv_driver);

    let sig_port = addr_list.get(0).unwrap().port();

    let sig_url = Tx5Url::new(format!("ws://localhost:{}", sig_port)).unwrap();
    println!("sig_url: {}", sig_url);

    let (ep1, _ep_rcv1) = Ep::new().await.unwrap();

    let cli_url1 = ep1.listen(sig_url.clone()).await.unwrap();

    println!("cli_url1: {}", cli_url1);

    let (ep2, _ep_rcv2) = Ep::new().await.unwrap();

    let cli_url2 = ep2.listen(sig_url).await.unwrap();

    println!("cli_url2: {}", cli_url2);

    let msg_sent = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let msg_sent2 = msg_sent.clone();

    // *Send* the message, but it shouldn't be received
    ep2.ban(cli_url1.id().unwrap(), std::time::Duration::from_secs(500));
    let fut = ep1.send(cli_url2.clone(), &b"hello"[..]);
    let task = tokio::task::spawn(async move {
        if fut.await.is_err() {
            // it's okay if this errors, that was the point
            return;
        }
        // the future resolved successfully, that's bad, the ban didn't work.
        msg_sent2.store(true, std::sync::atomic::Ordering::SeqCst);
    });

    // allow some time for it to be sent
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;

    // Now try banning the *sending* side. Should get an error sending.
    ep1.ban(cli_url2.id().unwrap(), std::time::Duration::from_secs(500));
    assert!(ep1.send(cli_url2, &b"hello"[..]).await.is_err());

    // Allow some additional time for the first send to connect / etc
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;

    // if the message sent, our ban didn't work
    if msg_sent.load(std::sync::atomic::Ordering::SeqCst) {
        panic!("message wast sent! ban failed");
    }

    task.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn large_messages() {
    init_tracing();

    let mut srv_config = tx5_signal_srv::Config::default();
    srv_config.port = 0;
    srv_config.demo = true;

    let (srv_driver, addr_list, _) =
        tx5_signal_srv::exec_tx5_signal_srv(srv_config).unwrap();
    tokio::task::spawn(srv_driver);

    let sig_port = addr_list.get(0).unwrap().port();

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

#[tokio::test(flavor = "multi_thread")]
async fn broadcast() {
    init_tracing();

    let mut srv_config = tx5_signal_srv::Config::default();
    srv_config.port = 0;
    srv_config.demo = true;

    let (srv_driver, addr_list, _) =
        tx5_signal_srv::exec_tx5_signal_srv(srv_config).unwrap();
    tokio::task::spawn(srv_driver);

    let sig_port = addr_list.get(0).unwrap().port();

    let sig_url = Tx5Url::new(format!("ws://localhost:{}", sig_port)).unwrap();
    println!("sig_url: {}", sig_url);

    let (ep1, mut ep_rcv1) = Ep::new().await.unwrap();
    let cli_url1 = ep1.listen(sig_url.clone()).await.unwrap();
    println!("cli_url1: {}", cli_url1);

    let (ep2, mut ep_rcv2) = Ep::new().await.unwrap();
    let cli_url2 = ep2.listen(sig_url.clone()).await.unwrap();
    println!("cli_url2: {}", cli_url2);

    let (ep3, mut ep_rcv3) = Ep::new().await.unwrap();
    let cli_url3 = ep3.listen(sig_url).await.unwrap();
    println!("cli_url3: {}", cli_url3);

    ep1.send(cli_url2.clone(), &b"hello"[..]).await.unwrap();
    ep1.send(cli_url3, &b"hello"[..]).await.unwrap();

    match ep_rcv1.recv().await {
        Some(Ok(EpEvt::Connected { .. })) => (),
        oth => panic!("unexpected: {:?}", oth),
    }

    match ep_rcv1.recv().await {
        Some(Ok(EpEvt::Connected { .. })) => (),
        oth => panic!("unexpected: {:?}", oth),
    }

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

    match ep_rcv3.recv().await {
        Some(Ok(EpEvt::Connected { .. })) => (),
        oth => panic!("unexpected: {:?}", oth),
    }

    let recv = ep_rcv3.recv().await;

    match recv {
        Some(Ok(EpEvt::Data {
            rem_cli_url, data, ..
        })) => {
            assert_eq!(cli_url1, rem_cli_url);
            assert_eq!(b"hello", data.to_vec().unwrap().as_slice());
        }
        oth => panic!("unexpected {:?}", oth),
    }

    ep1.broadcast(&b"bcast"[..]).await.unwrap();

    let recv = ep_rcv2.recv().await;

    match recv {
        Some(Ok(EpEvt::Data {
            rem_cli_url, data, ..
        })) => {
            assert_eq!(cli_url1, rem_cli_url);
            assert_eq!(b"bcast", data.to_vec().unwrap().as_slice());
        }
        oth => panic!("unexpected {:?}", oth),
    }

    let recv = ep_rcv3.recv().await;

    match recv {
        Some(Ok(EpEvt::Data {
            rem_cli_url, data, ..
        })) => {
            assert_eq!(cli_url1, rem_cli_url);
            assert_eq!(b"bcast", data.to_vec().unwrap().as_slice());
        }
        oth => panic!("unexpected {:?}", oth),
    }

    ep2.broadcast(&b"bcast2"[..]).await.unwrap();

    let recv = ep_rcv1.recv().await;

    match recv {
        Some(Ok(EpEvt::Data {
            rem_cli_url, data, ..
        })) => {
            assert_eq!(cli_url2, rem_cli_url);
            assert_eq!(b"bcast2", data.to_vec().unwrap().as_slice());
        }
        oth => panic!("unexpected {:?}", oth),
    }
}
