use crate::*;
use std::sync::{Arc, Mutex};

#[tokio::test(flavor = "multi_thread")]
async fn behavior_20_sec() {
    run_behavior(std::time::Duration::from_secs(20)).await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "this is a long-running test, `cargo test behavior -- --ignored`"]
async fn behavior_5_min() {
    run_behavior(std::time::Duration::from_secs(60 * 5)).await;
}

struct Share {
    pub sig_url: SigUrl,
    pub config: Arc<Config3>,
    pub tasks: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
    pub errors: Arc<Mutex<Vec<Error>>>,
}

impl Drop for Share {
    fn drop(&mut self) {
        eprintln!("BEG SHARE DROP");
        for task in self.tasks.lock().unwrap().drain(..) {
            task.abort();
        }
        eprintln!("END SHARE DROP");
    }
}

async fn run_behavior(dur: std::time::Duration) {
    let subscriber = tracing_subscriber::FmtSubscriber::builder()
        .with_env_filter(
            tracing_subscriber::filter::EnvFilter::from_default_env(),
        )
        .with_file(true)
        .with_line_number(true)
        .finish();

    let _ = tracing::subscriber::set_global_default(subscriber);

    let mut srv_config = tx5_signal_srv::Config::default();
    srv_config.port = 0;

    let (_sig_srv_hnd, addr_list, _) =
        tx5_signal_srv::exec_tx5_signal_srv(srv_config)
            .await
            .unwrap();

    let sig_port = addr_list.get(0).unwrap().port();

    let sig_url = SigUrl::new(format!("ws://localhost:{}", sig_port)).unwrap();

    tracing::info!(%sig_url);

    let mut config = Config3::default();
    config.timeout = std::time::Duration::from_secs(10);

    let share = Arc::new(Share {
        sig_url,
        config: Arc::new(config),
        tasks: Arc::new(Mutex::new(Vec::new())),
        errors: Arc::new(Mutex::new(Vec::new())),
    });

    let peer_echo = run_echo(&share).await;

    run_small_msg(share.clone(), peer_echo.clone());
    run_large_msg(share.clone(), peer_echo.clone());
    run_large_msg(share.clone(), peer_echo.clone());
    run_ban(share.clone(), peer_echo.clone());
    run_self_ban(share.clone(), peer_echo.clone());
    run_dropout(share.clone(), peer_echo);

    tokio::time::sleep(dur).await;

    {
        let lock = share.errors.lock().unwrap();
        if lock.len() > 0 {
            panic!("{:#?}", *lock);
        }
    }

    drop(share);
    eprintln!("TEST DROP");
}

macro_rules! track_err {
    ($e:ident, $t:literal, $b:block) => {
        if let Err(err) = $b {
            let err =
                Error::str(format!("{}:{}:{}: {err:?}", file!(), line!(), $t,))
                    .into();
            eprintln!("{err:?}");
            $e.lock().unwrap().push(err);
        }
    };
}

async fn run_echo(share: &Share) -> PeerUrl {
    let (ep, mut recv) = Ep3::new(share.config.clone()).await;
    let ep = Arc::new(ep);
    let url = ep.listen(share.sig_url.clone()).await.unwrap();
    let errors = share.errors.clone();

    share
        .tasks
        .lock()
        .unwrap()
        .push(tokio::task::spawn(async move {
            while let Some(evt) = recv.recv().await {
                if let Ep3Event::Message {
                    peer_url, message, ..
                } = evt
                {
                    track_err!(errors, "echo response", {
                        ep.send(peer_url.clone(), &message).await
                    });

                    if message == b"ban" {
                        let ep = ep.clone();
                        tokio::task::spawn(async move {
                            tokio::time::sleep(
                                std::time::Duration::from_millis(800),
                            )
                            .await;

                            ep.ban(
                                peer_url.id().unwrap(),
                                std::time::Duration::from_secs(11),
                            );
                        });
                    }
                }
            }
        }));

    url
}

fn run_small_msg(share: Arc<Share>, peer_echo: PeerUrl) {
    share
        .clone()
        .tasks
        .lock()
        .unwrap()
        .push(tokio::task::spawn(async move {
            let (ep, mut recv) = Ep3::new(share.config.clone()).await;
            ep.listen(share.sig_url.clone()).await.unwrap();
            let errors = share.errors.clone();
            drop(share);

            let mut msg_id: usize = 0;
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;

                msg_id += 1;
                let msg = format!("msg:{msg_id}");

                track_err!(errors, "small msg send", {
                    ep.send(peer_echo.clone(), msg.as_bytes()).await
                });

                loop {
                    match recv.recv().await.unwrap() {
                        Ep3Event::Connected { .. }
                        | Ep3Event::Disconnected { .. } => (),
                        Ep3Event::Message {
                            peer_url, message, ..
                        } => {
                            if peer_url == peer_echo {
                                let m = String::from_utf8_lossy(&message);
                                let mut p = m.split("msg:");
                                p.next().unwrap();
                                assert_eq!(
                                    msg_id,
                                    p.next().unwrap().parse::<usize>().unwrap()
                                );
                                eprintln!("small_msg success");
                                break;
                            }
                        }
                        oth => errors.lock().unwrap().push(
                            Error::str(format!("unexpected: {oth:?}",)).into(),
                        ),
                    }
                }
            }
        }));
}

fn run_large_msg(share: Arc<Share>, peer_echo: PeerUrl) {
    share
        .clone()
        .tasks
        .lock()
        .unwrap()
        .push(tokio::task::spawn(async move {
            let (ep, mut recv) = Ep3::new(share.config.clone()).await;
            ep.listen(share.sig_url.clone()).await.unwrap();
            let errors = share.errors.clone();
            drop(share);

            let mut full = vec![0; 15 * 1024 * 1024];

            loop {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;

                use rand::Rng;
                rand::thread_rng().fill(&mut full[..]);

                track_err!(errors, "small msg send", {
                    ep.send(peer_echo.clone(), &full).await
                });

                loop {
                    match recv.recv().await.unwrap() {
                        Ep3Event::Connected { .. }
                        | Ep3Event::Disconnected { .. } => (),
                        Ep3Event::Message {
                            peer_url, message, ..
                        } => {
                            if peer_url == peer_echo {
                                assert_eq!(full, message);
                                eprintln!("large_msg success");
                                break;
                            }
                        }
                        oth => errors.lock().unwrap().push(
                            Error::str(format!("unexpected: {oth:?}",)).into(),
                        ),
                    }
                }
            }
        }));
}

fn run_ban(share: Arc<Share>, peer_echo: PeerUrl) {
    share
        .clone()
        .tasks
        .lock()
        .unwrap()
        .push(tokio::task::spawn(async move {
            let (ep, mut recv) = Ep3::new(share.config.clone()).await;
            ep.listen(share.sig_url.clone()).await.unwrap();
            let errors = share.errors.clone();
            drop(share);

            loop {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;

                let time1 = std::time::Instant::now();

                eprintln!("ban req");

                if ep.send(peer_echo.clone(), b"ban").await.is_err() {
                    // we might get an error when the connection realizes
                    // it was shut down... just try again
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

                    track_err!(errors, "ban req send", {
                        ep.send(peer_echo.clone(), b"ban").await
                    });
                }

                let time2 = std::time::Instant::now();

                loop {
                    match recv.recv().await.unwrap() {
                        Ep3Event::Connected { .. }
                        | Ep3Event::Disconnected { .. } => (),
                        Ep3Event::Message {
                            peer_url, message, ..
                        } => {
                            if peer_url == peer_echo {
                                assert_eq!(b"ban", message.as_slice());
                                break;
                            }
                        }
                        oth => errors.lock().unwrap().push(
                            Error::str(format!("unexpected: {oth:?}",)).into(),
                        ),
                    }
                }

                eprintln!("BAN TIME1: {}", time1.elapsed().as_secs_f64());
                eprintln!("BAN TIME2: {}", time2.elapsed().as_secs_f64());

                tokio::time::sleep(std::time::Duration::from_secs(1)).await;

                let start = std::time::Instant::now();

                eprintln!("ban check");
                if ep.send(peer_echo.clone(), b"hello").await.is_ok() {
                    let e = Error::id("WasNotBanned").into();
                    eprintln!("{e:?}");
                    errors.lock().unwrap().push(e);
                } else {
                    eprintln!("ban success");
                }

                let sleep_for =
                    std::time::Duration::from_secs(10) - start.elapsed();

                tokio::time::sleep(sleep_for).await;
            }
        }));
}

fn run_self_ban(share: Arc<Share>, peer_echo: PeerUrl) {
    share
        .clone()
        .tasks
        .lock()
        .unwrap()
        .push(tokio::task::spawn(async move {
            let (ep, mut recv) = Ep3::new(share.config.clone()).await;
            ep.listen(share.sig_url.clone()).await.unwrap();
            let errors = share.errors.clone();
            drop(share);

            loop {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;

                track_err!(errors, "self_ban check send", {
                    ep.send(peer_echo.clone(), b"hello").await
                });

                loop {
                    match recv.recv().await.unwrap() {
                        Ep3Event::Connected { .. }
                        | Ep3Event::Disconnected { .. } => (),
                        Ep3Event::Message {
                            peer_url, message, ..
                        } => {
                            if peer_url == peer_echo {
                                assert_eq!(b"hello", message.as_slice());
                                break;
                            }
                        }
                        oth => errors.lock().unwrap().push(
                            Error::str(format!("unexpected: {oth:?}",)).into(),
                        ),
                    }
                }

                ep.ban(
                    peer_echo.id().unwrap(),
                    std::time::Duration::from_secs(11),
                );

                tokio::time::sleep(std::time::Duration::from_secs(1)).await;

                let start = std::time::Instant::now();

                eprintln!("self_ban check");
                if ep.send(peer_echo.clone(), b"hello").await.is_ok() {
                    let e = Error::id("WasNotBanned").into();
                    eprintln!("{e:?}");
                    errors.lock().unwrap().push(e);
                } else {
                    eprintln!("self_ban success");
                }

                let sleep_for =
                    std::time::Duration::from_secs(10) - start.elapsed();

                tokio::time::sleep(sleep_for).await;
            }
        }));
}

fn run_dropout(share: Arc<Share>, peer_echo: PeerUrl) {
    share
        .clone()
        .tasks
        .lock()
        .unwrap()
        .push(tokio::task::spawn(async move {
            let sig_url = share.sig_url.clone();
            let config = share.config.clone();
            let errors = share.errors.clone();
            drop(share);

            loop {
                let (ep, mut recv) = Ep3::new(config.clone()).await;
                ep.listen(sig_url.clone()).await.unwrap();

                tokio::time::sleep(std::time::Duration::from_secs(1)).await;

                track_err!(errors, "dropout check send", {
                    ep.send(peer_echo.clone(), b"hello").await
                });

                loop {
                    match recv.recv().await.unwrap() {
                        Ep3Event::Connected { .. }
                        | Ep3Event::Disconnected { .. } => (),
                        Ep3Event::Message {
                            peer_url, message, ..
                        } => {
                            if peer_url == peer_echo {
                                assert_eq!(b"hello", message.as_slice());
                                eprintln!("dropout success");
                                break;
                            }
                        }
                        oth => errors.lock().unwrap().push(
                            Error::str(format!("unexpected: {oth:?}",)).into(),
                        ),
                    }
                }

                drop(ep);
                drop(recv);

                tokio::time::sleep(std::time::Duration::from_secs(4)).await;
            }
        }));
}
