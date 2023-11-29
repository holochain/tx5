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
async fn check_send_backpressure() {
    init_tracing();

    let this_id = Id::from([1; 32]);
    let this_id = &this_id;
    let other_id = Id::from([2; 32]);
    let other_id = &other_id;

    let sig_url = Tx5Url::new("ws://fake:1").unwrap();

    let this_url = sig_url.to_client(this_id.clone());
    let this_url = &this_url;
    let other_url = sig_url.to_client(other_id.clone());
    let other_url = &other_url;

    let send_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));

    let notify = Arc::new(tokio::sync::Notify::new());

    let this_url_sig = this_url.clone();
    let other_id_sig = other_id.clone();
    let send_count_conn = send_count.clone();
    let notify_conn = notify.clone();
    let config = DefConfig::default()
        .with_max_send_bytes(20)
        .with_per_data_chan_buf_low(10)
        .with_new_sig_cb(move |_, _, seed| {
            let (sig_state, mut sig_evt) = seed
                .result_ok(
                    this_url_sig.clone(),
                    Arc::new(serde_json::json!([])),
                )
                .unwrap();
            let other_id = other_id_sig.clone();
            tokio::task::spawn(async move {
                while let Some(Ok(evt)) = sig_evt.recv().await {
                    match evt {
                        state::SigStateEvt::SndOffer(_, _, mut r) => {
                            r.send(Ok(()));
                            sig_state
                                .answer(
                                    other_id.clone(),
                                    BackBuf::from_slice(&[]).unwrap(),
                                )
                                .unwrap();
                        }
                        _ => println!("unhandled SIG_EVT: {evt:?}"),
                    }
                }
            });
        })
        .with_new_conn_cb(move |_, _, seed| {
            let (conn_state, mut conn_evt) = seed.result_ok().unwrap();
            let send_count_conn = send_count_conn.clone();
            let notify_conn = notify_conn.clone();
            tokio::task::spawn(async move {
                while let Some(Ok(evt)) = conn_evt.recv().await {
                    match evt {
                        state::ConnStateEvt::CreateOffer(mut r) => {
                            r.send(BackBuf::from_slice(&[]));
                        }
                        state::ConnStateEvt::SetLoc(_, mut r) => {
                            r.send(Ok(()));
                        }
                        state::ConnStateEvt::SetRem(_, mut r) => {
                            r.send(Ok(()));
                        }
                        state::ConnStateEvt::SndData(_, mut r) => {
                            let notify_conn = notify_conn.clone();
                            tokio::task::spawn(async move {
                                notify_conn.notified().await;
                                r.send(Ok(state::BufState::Low));
                            });
                            send_count_conn.fetch_add(
                                1,
                                std::sync::atomic::Ordering::SeqCst,
                            );
                        }
                        _ => println!("unhandled CONN_EVT: {evt:?}"),
                    }
                }
            });
            conn_state.ready().unwrap();
            tokio::task::spawn(async move {
                let _conn_state = conn_state;

                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            });
        });

    let (ep1, _ep_rcv1) = Ep::with_config(config).await.unwrap();

    ep1.listen(sig_url).await.unwrap();

    let fut1 = ep1.send(other_url.clone(), &b"1234567890"[..]);
    let send_task_1 = tokio::task::spawn(async move {
        fut1.await.unwrap();
    });

    let fut2 = ep1.send(other_url.clone(), &b"1234567890"[..]);
    let send_task_2 = tokio::task::spawn(async move {
        fut2.await.unwrap();
    });

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // make sure only the preflight and our first message have been "sent"
    assert_eq!(2, send_count.load(std::sync::atomic::Ordering::SeqCst));

    notify.notify_waiters();

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // after sending the preflight and first messages through,
    // now the second message is queued up for send on our mock backend.
    assert_eq!(3, send_count.load(std::sync::atomic::Ordering::SeqCst));

    // now let the second message through
    notify.notify_waiters();

    // make sure our send tasks resolve
    send_task_1.await.unwrap();
    send_task_2.await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn endpoint_sanity() {
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
        Some(Ok(EpEvt::Disconnected { .. })) => (),
        oth => panic!("unexpected: {:?}", oth),
    }

    match ep_rcv2.recv().await {
        Some(Ok(EpEvt::Connected { .. })) => (),
        Some(Ok(EpEvt::Disconnected { .. })) => (),
        oth => panic!("unexpected: {:?}", oth),
    }

    assert_eq!(2, valid_count.load(std::sync::atomic::Ordering::SeqCst));
}

#[tokio::test(flavor = "multi_thread")]
async fn preflight_huge() {
    init_tracing();

    const HUGE_DATA: bytes::Bytes =
        bytes::Bytes::from_static(&[42; 16 * 1024 * 512]);

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

    fn make_config(
        ep_num: usize,
        valid_count: Arc<std::sync::atomic::AtomicUsize>,
    ) -> DefConfig {
        DefConfig::default()
            .with_conn_preflight(move |_, _| {
                println!("PREFLIGHT:{ep_num}");
                Box::pin(async move { Ok(Some(HUGE_DATA.clone())) })
            })
            .with_conn_validate(move |_, _, data| {
                println!("VALIDATE:{ep_num}");
                assert_eq!(HUGE_DATA, data.unwrap());
                valid_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Box::pin(async move { Ok(()) })
            })
    }

    let (ep1, mut ep_rcv1) =
        Ep::with_config(make_config(1, valid_count.clone()))
            .await
            .unwrap();
    let cli_url1 = ep1.listen(sig_url.clone()).await.unwrap();
    println!("cli_url1: {}", cli_url1);

    let (ep2, mut ep_rcv2) =
        Ep::with_config(make_config(2, valid_count.clone()))
            .await
            .unwrap();
    let cli_url2 = ep2.listen(sig_url).await.unwrap();
    println!("cli_url2: {}", cli_url2);

    let (s1, r1) = tokio::sync::oneshot::channel();
    let mut s1 = Some(s1);
    tokio::task::spawn(async move {
        loop {
            match ep_rcv1.recv().await {
                Some(Ok(EpEvt::Connected { .. })) => {
                    if let Some(s1) = s1.take() {
                        println!("ONE connected");
                        let _ = s1.send(());
                    }
                }
                Some(Ok(EpEvt::Disconnected { .. })) => (),
                oth => panic!("unexpected: {:?}", oth),
            }
        }
    });

    let (s2, r2) = tokio::sync::oneshot::channel();
    let mut s2 = Some(s2);
    tokio::task::spawn(async move {
        let mut done_count = 0;
        let done_count = &mut done_count;
        let mut check = move || {
            *done_count += 1;
            println!("TWO recv: check_count: {done_count}");
            if *done_count == 2 {
                if let Some(s2) = s2.take() {
                    let _ = s2.send(());
                }
            }
        };

        loop {
            let _ = ep_rcv2.recv().await;
            check();
        }
    });

    ep1.send(cli_url2, &b"hello"[..]).await.unwrap();

    r1.await.unwrap();
    r2.await.unwrap();

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

    use rand::Rng;
    let mut rng = rand::thread_rng();
    let mut msg_1 = vec![0; 1024 * 1024 * 15];
    rng.fill(&mut msg_1[..]);
    let mut msg_2 = vec![0; 1024 * 58];
    rng.fill(&mut msg_2[..]);
    let msg_1_r = msg_1.clone();
    let msg_2_r = msg_2.clone();

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

    let recv_task1 = tokio::task::spawn(async move {
        match ep_rcv1.recv().await {
            Some(Ok(EpEvt::Connected { .. })) => (),
            oth => panic!("unexpected: {oth:?}"),
        }

        println!("ep1 connected");

        // TODO - connections not getting closed properly
        /*
        match ep_rcv1.recv().await {
            Some(Ok(EpEvt::Disconnected { .. })) => (),
            oth => panic!("unexpected: {oth:?}"),
        }

        println!("ep1 disconnected");
        */
    });

    let cli_url1 = ep1.listen(sig_url.clone()).await.unwrap();

    println!("cli_url1: {}", cli_url1);

    let (ep2, mut ep_rcv2) = Ep::new().await.unwrap();

    let cli_url2 = ep2.listen(sig_url).await.unwrap();

    println!("cli_url2: {}", cli_url2);

    let recv_task2 = {
        tokio::task::spawn(async move {
            match ep_rcv2.recv().await {
                Some(Ok(EpEvt::Connected { .. })) => (),
                oth => panic!("unexpected: {oth:?}"),
            }

            for _ in 0..2 {
                let recv = ep_rcv2.recv().await;
                match recv {
                    Some(Ok(EpEvt::Data {
                        rem_cli_url, data, ..
                    })) => {
                        assert_eq!(cli_url1, rem_cli_url);
                        let msg = data.to_vec().unwrap();
                        if msg.len() == msg_1_r.len() {
                            assert_eq!(msg, msg_1_r);
                        } else if msg.len() == msg_2_r.len() {
                            assert_eq!(msg, msg_2_r);
                        } else {
                            panic!("unexpected");
                        }
                    }
                    oth => panic!("unexpected {:?}", oth),
                }
            }
        })
    };

    let f1 = ep1.send(cli_url2.clone(), msg_1.as_slice());
    let f2 = ep1.send(cli_url2, msg_2.as_slice());

    tokio::try_join!(f1, f2).unwrap();

    recv_task1.await.unwrap();
    recv_task2.await.unwrap();
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
