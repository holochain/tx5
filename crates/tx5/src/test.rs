use super::*;
use std::sync::Arc;

const HUGE_DATA: bytes::Bytes =
    bytes::Bytes::from_static(&[42; 16 * 1024 * 512]);

const SMALL_DATA: bytes::Bytes = bytes::Bytes::from_static(&[42; 16]);

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

    let (pend_send, mut pend_recv) = tokio::sync::mpsc::unbounded_channel();

    let this_url_sig = this_url.clone();
    let other_id_sig = other_id.clone();
    let send_count_conn = send_count.clone();
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
            let pend_send = pend_send.clone();
            tokio::task::spawn(async move {
                while let Some(Ok(evt)) = conn_evt.recv().await {
                    match evt {
                        state::ConnStateEvt::CreateOffer(mut resp) => {
                            resp.send(BackBuf::from_slice(&[]));
                        }
                        state::ConnStateEvt::SetLoc(_, mut resp) => {
                            resp.send(Ok(()));
                        }
                        state::ConnStateEvt::SetRem(_, mut resp) => {
                            resp.send(Ok(()));
                        }
                        state::ConnStateEvt::SndData(_, mut resp) => {
                            let (s, r) = tokio::sync::oneshot::channel();
                            let _ = pend_send.send(s);
                            tokio::task::spawn(async move {
                                let _ = r.await;
                                resp.send(Ok(state::BufState::Low));
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
            // leak this
            std::mem::forget(conn_state);
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

    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;

            if send_count.load(std::sync::atomic::Ordering::SeqCst) >= 2 {
                break;
            }
        }
    })
    .await
    .unwrap();

    // wait to see if a third message comes through
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    // make sure only the preflight and our first message have been "sent"
    assert_eq!(2, send_count.load(std::sync::atomic::Ordering::SeqCst));

    // let the preflight message through
    let _ = pend_recv.recv().await.unwrap().send(());

    // let the first message through
    let _ = pend_recv.recv().await.unwrap().send(());

    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;

            if send_count.load(std::sync::atomic::Ordering::SeqCst) >= 3 {
                break;
            }
        }
    })
    .await
    .unwrap();

    // after sending the preflight and first messages through,
    // now the second message is queued up for send on our mock backend.
    assert_eq!(3, send_count.load(std::sync::atomic::Ordering::SeqCst));

    // now let the second message through
    let _ = pend_recv.recv().await.unwrap().send(());

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

    fn make_config(
        valid_count: Arc<std::sync::atomic::AtomicUsize>,
    ) -> DefConfig {
        DefConfig::default()
            .with_conn_preflight(|_, _| {
                println!("PREFLIGHT");
                Box::pin(async move { Ok(Some(SMALL_DATA.clone())) })
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

    ep_rcv1.recv().await.unwrap().unwrap();
    ep_rcv2.recv().await.unwrap().unwrap();

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

    ep1.send(cli_url2, &b"hello"[..]).await.unwrap();

    ep_rcv1.recv().await.unwrap().unwrap();
    ep_rcv2.recv().await.unwrap().unwrap();

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

    let recv_task = tokio::task::spawn(async move {
        let mut got_one = false;
        let mut got_two = false;

        loop {
            let recv = ep_rcv2.recv().await;
            match recv {
                Some(Ok(EpEvt::Data {
                    rem_cli_url, data, ..
                })) => {
                    assert_eq!(cli_url1, rem_cli_url);
                    let msg = data.to_vec().unwrap();
                    if msg.len() == HUGE_DATA.len() {
                        assert_eq!(msg, HUGE_DATA);
                        got_one = true;
                    } else if msg.len() == SMALL_DATA.len() {
                        assert_eq!(msg, SMALL_DATA);
                        got_two = true;
                    } else {
                        panic!("unexpected");
                    }
                }
                _ => (),
            }
            if got_one && got_two {
                return;
            }
        }
    });

    let f1 = ep1.send(cli_url2.clone(), HUGE_DATA.clone());
    let f2 = ep1.send(cli_url2, SMALL_DATA.clone());

    tokio::try_join!(f1, f2).unwrap();

    recv_task.await.unwrap();
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
