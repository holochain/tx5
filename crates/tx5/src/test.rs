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

    println!(
        "{}",
        serde_json::to_string_pretty(&ep1.get_stats().await.unwrap()).unwrap()
    );
    println!(
        "{}",
        serde_json::to_string_pretty(&ep2.get_stats().await.unwrap()).unwrap()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ban() {
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

#[tokio::test(flavor = "multi_thread")]
async fn broadcast() {
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
