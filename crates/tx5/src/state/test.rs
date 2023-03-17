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

struct Test {
    shutdown: bool,
    cli_a: Tx5Url,
    id_a: Id,
    cli_b: Tx5Url,
    id_b: Id,
    state: State,
    state_evt: ManyRcv<StateEvt>,
    sig_state: SigState,
    sig_evt: ManyRcv<SigStateEvt>,
}

impl Drop for Test {
    fn drop(&mut self) {
        if !self.shutdown {
            // print and abort, since panic within drop breaks things
            eprintln!("Expected Test::shutdown() to be called, aborting. Backtrace: {:#?}", std::backtrace::Backtrace::capture());
            std::process::abort();
        }
    }
}

impl Test {
    pub async fn new(as_a: bool) -> Self {
        let sig: Tx5Url = Tx5Url::new("wss://s").unwrap();
        let cli_a: Tx5Url = Tx5Url::new(
            "wss://s/tx5-ws/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        )
        .unwrap();
        let id_a = cli_a.id().unwrap();
        let cli_b: Tx5Url = Tx5Url::new(
            "wss://s/tx5-ws/BAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        )
        .unwrap();
        let id_b = cli_b.id().unwrap();

        let config = DefConfig::default().into_config().await.unwrap();
        let (state, mut state_evt) = State::new(config).unwrap();

        // -- register with a signal server -- //

        let task = {
            let state = state.clone();
            let sig = sig.clone();

            // can't do this inline, since it won't resolve until result_ok
            // call on the seed below
            tokio::task::spawn(
                async move { state.listener_sig(sig).await.unwrap() },
            )
        };

        let sig_seed = match state_evt.recv().await {
            Some(Ok(StateEvt::NewSig(_url, seed))) => seed,
            oth => panic!("unexpected: {:?}", oth),
        };

        let cli = if as_a { cli_a.clone() } else { cli_b.clone() };
        let (sig_state, sig_evt) = sig_seed
            .result_ok(cli, Arc::new(serde_json::json!([])))
            .unwrap();

        task.await.unwrap();

        if as_a {
            assert!(matches!(
                state_evt.recv().await,
                Some(Ok(StateEvt::Address(tmp))) if tmp == cli_a,
            ));
        } else {
            assert!(matches!(
                state_evt.recv().await,
                Some(Ok(StateEvt::Address(tmp))) if tmp == cli_b,
            ));
        }

        println!("got addr");

        Self {
            shutdown: false,
            cli_a,
            id_a,
            cli_b,
            id_b,
            state,
            state_evt,
            sig_state,
            sig_evt,
        }
    }

    pub async fn shutdown(mut self) {
        self.shutdown = true;

        let enc = prometheus::TextEncoder::new();
        let mut buf = Vec::new();
        use prometheus::Encoder;
        enc.encode(&prometheus::default_registry().gather(), &mut buf)
            .unwrap();
        println!("{}", String::from_utf8_lossy(&buf));

        self.state.close(Error::id("TestShutdown"));

        let res = self.state_evt.recv().await;
        assert!(
            matches!(res, Some(Ok(StateEvt::Disconnected { .. })),),
            "expected Disconnected, got: {:?}",
            res
        );

        let res = self.state_evt.recv().await;
        assert!(
            matches!(
                res,
                Some(Err(ref err)) if &err.to_string() == "TestShutdown",
            ),
            "expected Err(\"TestShutdown\"), got: {:?}",
            res
        );

        // erm... is this what we want??
        assert!(matches!(
            self.state_evt.recv().await,
            Some(Err(err)) if &err.to_string() == "Dropped",
        ));

        assert!(matches!(self.state_evt.recv().await, None));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn extended_outgoing() {
    init_tracing();

    let mut test = Test::new(true).await;

    // -- send data to a "peer" (causes connecting to that peer) -- //

    let task = {
        let state = test.state.clone();
        let cli_b = test.cli_b.clone();

        tokio::task::spawn(async move {
            state.snd_data(cli_b.clone(), &b"hello"[..]).await.unwrap()
        })
    };

    // -- new peer connection -- //

    let conn_seed = match test.state_evt.recv().await {
        Some(Ok(StateEvt::NewConn(_ice_servers, seed))) => seed,
        oth => panic!("unexpected: {:?}", oth),
    };

    println!("got new conn");

    let (conn_state, mut conn_evt) = conn_seed.result_ok().unwrap();

    // -- generate an offer -- //

    let mut resp = match conn_evt.recv().await {
        Some(Ok(ConnStateEvt::CreateOffer(resp))) => resp,
        oth => panic!("unexpected: {:?}", oth),
    };

    resp.send(BackBuf::from_slice(b"offer"));

    println!("got create_offer");

    match test.sig_evt.recv().await {
        Some(Ok(SigStateEvt::SndOffer(id, mut buf, mut resp))) => {
            assert_eq!(id, test.id_b);
            assert_eq!(&buf.to_vec().unwrap(), b"offer");
            resp.send(Ok(()));
        }
        oth => panic!("unexpected: {:?}", oth),
    }

    println!("sent offer");

    match conn_evt.recv().await {
        Some(Ok(ConnStateEvt::SetLoc(mut offer, mut resp))) => {
            assert_eq!(&offer.to_vec().unwrap(), b"offer");
            resp.send(Ok(()));
        }
        oth => panic!("unexpected: {:?}", oth),
    }

    println!("set loc");

    test.sig_state
        .answer(test.id_b, BackBuf::from_slice(b"answer").unwrap())
        .unwrap();

    match conn_evt.recv().await {
        Some(Ok(ConnStateEvt::SetRem(mut answer, mut resp))) => {
            assert_eq!(&answer.to_vec().unwrap(), b"answer");
            resp.send(Ok(()));
        }
        oth => panic!("unexpected: {:?}", oth),
    };

    println!("set rem");

    conn_state
        .ice(BackBuf::from_slice(b"ice").unwrap())
        .unwrap();

    match test.sig_evt.recv().await {
        Some(Ok(SigStateEvt::SndIce(id, mut buf, mut resp))) => {
            assert_eq!(id, test.id_b);
            assert_eq!(&buf.to_vec().unwrap(), b"ice");
            resp.send(Ok(()));
        }
        oth => panic!("unexpected: {:?}", oth),
    }

    test.sig_state
        .ice(test.id_b, BackBuf::from_slice(b"rem_ice").unwrap())
        .unwrap();

    println!("sent ice");

    match conn_evt.recv().await {
        Some(Ok(ConnStateEvt::SetIce(mut ice, mut resp))) => {
            assert_eq!(&ice.to_vec().unwrap(), b"rem_ice");
            resp.send(Ok(()));
        }
        oth => panic!("unexpected: {:?}", oth),
    };

    println!("set rem ice");

    conn_state.ready().unwrap();

    println!("ready");

    match test.state_evt.recv().await {
        Some(Ok(StateEvt::Connected { .. })) => (),
        oth => panic!("unexpected: {:?}", oth),
    }

    match conn_evt.recv().await {
        Some(Ok(ConnStateEvt::SndData(mut data, mut resp))) => {
            assert_eq!(&data.to_vec().unwrap()[8..], b"hello");
            resp.send(Ok(BufState::Low));
        }
        oth => panic!("unexpected: {:?}", oth),
    };

    println!("snd data");

    task.await.unwrap();

    // -- recv data from the remote -- //

    println!("about to rcv");

    conn_state
        .rcv_data(BackBuf::from_slice(b"\x01\0\0\0\0\0\0\0world").unwrap())
        .unwrap();

    conn_state
        .rcv_data(BackBuf::from_slice(b"\x01\0\0\0\0\0\0\0").unwrap())
        .unwrap();

    match test.state_evt.recv().await {
        Some(Ok(StateEvt::RcvData(url, data, _permit))) => {
            assert_eq!(url, test.cli_b);
            assert_eq!(&data.to_vec().unwrap(), b"world");
        }
        oth => panic!("unexpected: {:?}", oth),
    };

    println!("rcv data");

    test.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn short_incoming() {
    let mut test = Test::new(true).await;

    // -- receive an incoming offer -- //

    test.sig_state
        .offer(test.id_b, BackBuf::from_slice(b"offer").unwrap())
        .unwrap();

    // -- new peer connection -- //

    let conn_seed = match test.state_evt.recv().await {
        Some(Ok(StateEvt::NewConn(_ice_servers, seed))) => seed,
        oth => panic!("unexpected: {:?}", oth),
    };

    println!("got new conn");

    let (_conn_state, mut conn_evt) = conn_seed.result_ok().unwrap();

    match conn_evt.recv().await {
        Some(Ok(ConnStateEvt::SetRem(mut offer, mut resp))) => {
            assert_eq!(&offer.to_vec().unwrap(), b"offer");
            resp.send(Ok(()));
        }
        oth => panic!("unexpected: {:?}", oth),
    };

    println!("set rem");

    let mut resp = match conn_evt.recv().await {
        Some(Ok(ConnStateEvt::CreateAnswer(resp))) => resp,
        oth => panic!("unexpected {:?}", oth),
    };

    resp.send(BackBuf::from_slice(b"answer"));

    println!("got create_answer");

    match conn_evt.recv().await {
        Some(Ok(ConnStateEvt::SetLoc(mut answer, mut resp))) => {
            assert_eq!(&answer.to_vec().unwrap(), b"answer");
            resp.send(Ok(()));
        }
        oth => panic!("unexpected: {:?}", oth),
    };

    println!("set loc");

    match test.sig_evt.recv().await {
        Some(Ok(SigStateEvt::SndAnswer(id, mut buf, mut resp))) => {
            assert_eq!(id, test.id_b);
            assert_eq!(&buf.to_vec().unwrap(), b"answer");
            resp.send(Ok(()));
        }
        oth => panic!("unexpected: {:?}", oth),
    }

    println!("sent answer");

    test.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn polite_in_offer() {
    let mut test = Test::new(true).await;

    // -- send data to a "peer" (causes connecting to that peer) -- //

    let task = {
        let state = test.state.clone();
        let cli_b = test.cli_b.clone();

        tokio::task::spawn(async move {
            state.snd_data(cli_b.clone(), &b"hello"[..]).await.unwrap()
        })
    };

    let conn_seed = match test.state_evt.recv().await {
        Some(Ok(StateEvt::NewConn(_ice_servers, seed))) => seed,
        oth => panic!("unexpected: {:?}", oth),
    };

    println!("got new conn");

    let (_conn_state, mut conn_evt) = conn_seed.result_ok().unwrap();

    // -- generate an offer -- //

    let mut resp = match conn_evt.recv().await {
        Some(Ok(ConnStateEvt::CreateOffer(resp))) => resp,
        oth => panic!("unexpected: {:?}", oth),
    };

    resp.send(BackBuf::from_slice(b"offer"));

    println!("got create_offer");

    match test.sig_evt.recv().await {
        Some(Ok(SigStateEvt::SndOffer(id, mut buf, mut resp))) => {
            assert_eq!(id, test.id_b);
            assert_eq!(&buf.to_vec().unwrap(), b"offer");
            resp.send(Ok(()));
        }
        oth => panic!("unexpected: {:?}", oth),
    }

    println!("sent offer");

    match conn_evt.recv().await {
        Some(Ok(ConnStateEvt::SetLoc(mut offer, mut resp))) => {
            assert_eq!(&offer.to_vec().unwrap(), b"offer");
            resp.send(Ok(()));
        }
        oth => panic!("unexpected: {:?}", oth),
    }

    println!("set loc");

    // - BUT, instead we get an new incoming OFFER
    //   maybe because the other node started a racy try to connect to us too?

    test.sig_state
        .offer(test.id_b, BackBuf::from_slice(b"in_offer").unwrap())
        .unwrap();

    match conn_evt.recv().await {
        Some(Err(err)) => {
            assert_eq!("PoliteShutdownToAcceptIncomingOffer", &err.to_string())
        }
        oth => panic!("unexpected: {:?}", oth),
    }

    match test.state_evt.recv().await {
        Some(Ok(StateEvt::Disconnected { .. })) => (),
        oth => panic!("unexpected: {:?}", oth),
    }

    let conn_seed = match test.state_evt.recv().await {
        Some(Ok(StateEvt::NewConn(_ice_servers, seed))) => seed,
        oth => panic!("unexpected: {:?}", oth),
    };

    println!("got new conn");

    let (conn_state, mut conn_evt) = conn_seed.result_ok().unwrap();

    match conn_evt.recv().await {
        Some(Ok(ConnStateEvt::SetRem(mut offer, mut resp))) => {
            assert_eq!(&offer.to_vec().unwrap(), b"in_offer");
            resp.send(Ok(()));
        }
        oth => panic!("unexpected: {:?}", oth),
    };

    println!("set rem");

    let mut resp = match conn_evt.recv().await {
        Some(Ok(ConnStateEvt::CreateAnswer(resp))) => resp,
        oth => panic!("unexpected {:?}", oth),
    };

    resp.send(BackBuf::from_slice(b"answer"));

    println!("got create_answer");

    match conn_evt.recv().await {
        Some(Ok(ConnStateEvt::SetLoc(mut answer, mut resp))) => {
            assert_eq!(&answer.to_vec().unwrap(), b"answer");
            resp.send(Ok(()));
        }
        oth => panic!("unexpected: {:?}", oth),
    };

    println!("set loc");

    match test.sig_evt.recv().await {
        Some(Ok(SigStateEvt::SndAnswer(id, mut buf, mut resp))) => {
            assert_eq!(id, test.id_b);
            assert_eq!(&buf.to_vec().unwrap(), b"answer");
            resp.send(Ok(()));
        }
        oth => panic!("unexpected: {:?}", oth),
    }

    println!("sent answer");

    conn_state.ready().unwrap();

    println!("ready");

    match test.state_evt.recv().await {
        Some(Ok(StateEvt::Connected { .. })) => (),
        oth => panic!("unexpected: {:?}", oth),
    }

    match conn_evt.recv().await {
        Some(Ok(ConnStateEvt::SndData(mut data, mut resp))) => {
            assert_eq!(&data.to_vec().unwrap()[8..], b"hello");
            resp.send(Ok(BufState::Low));
        }
        oth => panic!("unexpected: {:?}", oth),
    };

    println!("snd data");

    // finally the data is sent
    task.await.unwrap();

    test.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn impolite_in_offer() {
    let mut test = Test::new(false).await;

    // -- send data to a "peer" (causes connecting to that peer) -- //

    let task = {
        let state = test.state.clone();
        let cli_a = test.cli_a.clone();

        tokio::task::spawn(async move {
            state.snd_data(cli_a.clone(), &b"hello"[..]).await.unwrap()
        })
    };

    let conn_seed = match test.state_evt.recv().await {
        Some(Ok(StateEvt::NewConn(_ice_servers, seed))) => seed,
        oth => panic!("unexpected: {:?}", oth),
    };

    println!("got new conn");

    let (conn_state, mut conn_evt) = conn_seed.result_ok().unwrap();

    // -- generate an offer -- //

    let mut resp = match conn_evt.recv().await {
        Some(Ok(ConnStateEvt::CreateOffer(resp))) => resp,
        oth => panic!("unexpected: {:?}", oth),
    };

    resp.send(BackBuf::from_slice(b"offer"));

    println!("got create_offer");

    match test.sig_evt.recv().await {
        Some(Ok(SigStateEvt::SndOffer(id, mut buf, mut resp))) => {
            assert_eq!(id, test.id_a);
            assert_eq!(&buf.to_vec().unwrap(), b"offer");
            resp.send(Ok(()));
        }
        oth => panic!("unexpected: {:?}", oth),
    }

    println!("sent offer");

    match conn_evt.recv().await {
        Some(Ok(ConnStateEvt::SetLoc(mut offer, mut resp))) => {
            assert_eq!(&offer.to_vec().unwrap(), b"offer");
            resp.send(Ok(()));
        }
        oth => panic!("unexpected: {:?}", oth),
    }

    println!("set loc");

    // - BUT, instead we get an new incoming OFFER
    //   maybe because the other node started a racy try to connect to us too?

    test.sig_state
        .offer(test.id_a, BackBuf::from_slice(b"in_offer").unwrap())
        .unwrap();

    // since we're the IMPOLITE node, we just ignore this offer
    // and continue with the negotiation of the original connection.

    test.sig_state
        .answer(test.id_a, BackBuf::from_slice(b"answer").unwrap())
        .unwrap();

    match conn_evt.recv().await {
        Some(Ok(ConnStateEvt::SetRem(mut answer, mut resp))) => {
            assert_eq!(&answer.to_vec().unwrap(), b"answer");
            resp.send(Ok(()));
        }
        oth => panic!("unexpected: {:?}", oth),
    };

    println!("set rem");

    conn_state.ready().unwrap();

    println!("ready");

    match test.state_evt.recv().await {
        Some(Ok(StateEvt::Connected { .. })) => (),
        oth => panic!("unexpected: {:?}", oth),
    }

    match conn_evt.recv().await {
        Some(Ok(ConnStateEvt::SndData(mut data, mut resp))) => {
            assert_eq!(&data.to_vec().unwrap()[8..], b"hello");
            resp.send(Ok(BufState::Low));
        }
        oth => panic!("unexpected: {:?}", oth),
    };

    println!("snd data");

    // finally the data is sent
    task.await.unwrap();

    test.shutdown().await;
}
