use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn extended_outgoing() {
    let (state, mut state_evt) = State::new();

    let sig_a: Tx4Url = Tx4Url::new("wss://a").unwrap();
    assert!(sig_a.is_server());
    assert!(!sig_a.is_client());
    assert!(sig_a.id().is_none());
    assert_eq!("a:443", &sig_a.endpoint());

    // -- register with a signal server -- //

    let task = {
        let state = state.clone();
        let sig_a = sig_a.clone();
        // can't do this inline, since it won't resolve until result_ok
        // call on the seed below
        tokio::task::spawn(
            async move { state.listener_sig(sig_a).await.unwrap() },
        )
    };

    let sig_seed = match state_evt.recv().await {
        Some(Ok(StateEvt::NewSig(seed))) => seed,
        _ => panic!("unexpected"),
    };

    let cli_a: Tx4Url = Tx4Url::new(
        "wss://a/tx4-ws/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
    )
    .unwrap();

    let (sig_state, mut sig_evt) = sig_seed.result_ok(cli_a.clone()).unwrap();

    task.await.unwrap();

    assert!(matches!(
        state_evt.recv().await,
        Some(Ok(StateEvt::Address(tmp))) if tmp == cli_a,
    ));

    println!("got addr");

    // -- send data to a "peer" (causes connecting to that peer) -- //

    let cli_b: Tx4Url = Tx4Url::new(
        "wss://a/tx4-ws/BAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
    )
    .unwrap();
    assert!(!cli_b.is_server());
    assert!(cli_b.is_client());
    let id_b = cli_b.id().unwrap();

    let task = {
        let state = state.clone();
        let cli_b = cli_b.clone();

        tokio::task::spawn(async move {
            state
                .snd_data(cli_b.clone(), Buf::from_slice(b"hello").unwrap())
                .await
                .unwrap()
        })
    };

    // -- new peer connection -- //

    let conn_seed = match state_evt.recv().await {
        Some(Ok(StateEvt::NewConn(seed))) => seed,
        _ => panic!("unexpected"),
    };

    println!("got new conn");

    let (conn_state, mut conn_evt) = conn_seed.result_ok().unwrap();

    // -- generate an offer -- //

    let mut resp = match conn_evt.recv().await {
        Some(Ok(ConnStateEvt::CreateOffer(resp))) => resp,
        _ => panic!("unexpected"),
    };

    resp.send(Buf::from_slice(b"offer"));

    println!("got create_offer");

    match sig_evt.recv().await {
        Some(Ok(SigStateEvt::SndOffer(id, mut buf, mut resp))) => {
            assert_eq!(id, id_b);
            assert_eq!(&buf.to_vec().unwrap(), b"offer");
            resp.send(Ok(()));
        }
        _ => panic!("unexpected"),
    }

    println!("sent offer");

    match conn_evt.recv().await {
        Some(Ok(ConnStateEvt::SetLoc(mut offer, mut resp))) => {
            assert_eq!(&offer.to_vec().unwrap(), b"offer");
            resp.send(Ok(()));
        }
        _ => panic!("unexpected"),
    }

    println!("set loc");

    sig_state
        .answer(id_b, Buf::from_slice(b"answer").unwrap())
        .unwrap();

    match conn_evt.recv().await {
        Some(Ok(ConnStateEvt::SetRem(mut answer, mut resp))) => {
            assert_eq!(&answer.to_vec().unwrap(), b"answer");
            resp.send(Ok(()));
        }
        _ => panic!("unexpected"),
    };

    println!("set rem");

    conn_state.ice(Buf::from_slice(b"ice").unwrap()).unwrap();

    match sig_evt.recv().await {
        Some(Ok(SigStateEvt::SndIce(id, mut buf, mut resp))) => {
            assert_eq!(id, id_b);
            assert_eq!(&buf.to_vec().unwrap(), b"ice");
            resp.send(Ok(()));
        }
        _ => panic!("unexpected"),
    }

    sig_state
        .ice(id_b, Buf::from_slice(b"rem_ice").unwrap())
        .unwrap();

    println!("sent ice");

    match conn_evt.recv().await {
        Some(Ok(ConnStateEvt::SetIce(mut ice, mut resp))) => {
            assert_eq!(&ice.to_vec().unwrap(), b"rem_ice");
            resp.send(Ok(()));
        }
        _ => panic!("unexpected"),
    };

    println!("set rem ice");

    conn_state.ready().unwrap();

    println!("ready");

    match conn_evt.recv().await {
        Some(Ok(ConnStateEvt::SndData(mut data, mut resp))) => {
            assert_eq!(&data.to_vec().unwrap(), b"hello");
            resp.send(Ok(BufState::Low));
        }
        _ => panic!("unexpected"),
    };

    println!("snd data");

    task.await.unwrap();

    // -- recv data from the remote -- //

    conn_state
        .rcv_data(Buf::from_slice(b"world").unwrap())
        .unwrap();

    match state_evt.recv().await {
        Some(Ok(StateEvt::RcvData(url, mut data, _permit))) => {
            assert_eq!(url, cli_b);
            assert_eq!(&data.to_vec().unwrap(), b"world");
        }
        _ => panic!("unexpected"),
    };

    println!("rcv data");

    // -- cleanup -- //

    state.close(Error::id("TestShutdown"));

    assert!(matches!(
        state_evt.recv().await,
        Some(Err(err)) if &err.to_string() == "TestShutdown",
    ));

    // erm... is this what we want??
    assert!(matches!(
        state_evt.recv().await,
        Some(Err(err)) if &err.to_string() == "Dropped",
    ));

    assert!(matches!(state_evt.recv().await, None));
}

#[tokio::test(flavor = "multi_thread")]
async fn short_incoming() {
    let (state, mut state_evt) = State::new();

    let sig_a: Tx4Url = Tx4Url::new("wss://a").unwrap();
    assert!(sig_a.is_server());
    assert!(!sig_a.is_client());
    assert!(sig_a.id().is_none());
    assert_eq!("a:443", &sig_a.endpoint());

    // -- register with a signal server -- //

    let task = {
        let state = state.clone();
        let sig_a = sig_a.clone();
        // can't do this inline, since it won't resolve until result_ok
        // call on the seed below
        tokio::task::spawn(
            async move { state.listener_sig(sig_a).await.unwrap() },
        )
    };

    let sig_seed = match state_evt.recv().await {
        Some(Ok(StateEvt::NewSig(seed))) => seed,
        _ => panic!("unexpected"),
    };

    let cli_a: Tx4Url = Tx4Url::new(
        "wss://a/tx4-ws/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
    )
    .unwrap();

    let (sig_state, mut sig_evt) = sig_seed.result_ok(cli_a.clone()).unwrap();

    task.await.unwrap();

    assert!(matches!(
        state_evt.recv().await,
        Some(Ok(StateEvt::Address(tmp))) if tmp == cli_a,
    ));

    println!("got addr");

    // -- receive an incoming offer -- //

    let cli_b: Tx4Url = Tx4Url::new(
        "wss://a/tx4-ws/BAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
    )
    .unwrap();
    assert!(!cli_b.is_server());
    assert!(cli_b.is_client());
    let id_b = cli_b.id().unwrap();

    sig_state
        .offer(id_b, Buf::from_slice(b"offer").unwrap())
        .unwrap();

    // -- new peer connection -- //

    let conn_seed = match state_evt.recv().await {
        Some(Ok(StateEvt::NewConn(seed))) => seed,
        _ => panic!("unexpected"),
    };

    println!("got new conn");

    let (_conn_state, mut conn_evt) = conn_seed.result_ok().unwrap();

    match conn_evt.recv().await {
        Some(Ok(ConnStateEvt::SetRem(mut offer, mut resp))) => {
            assert_eq!(&offer.to_vec().unwrap(), b"offer");
            resp.send(Ok(()));
        }
        _ => panic!("unexpected"),
    };

    println!("set rem");

    let mut resp = match conn_evt.recv().await {
        Some(Ok(ConnStateEvt::CreateAnswer(resp))) => resp,
        oth => panic!("unexpected {:?}", oth),
    };

    resp.send(Buf::from_slice(b"answer"));

    println!("got create_answer");

    match conn_evt.recv().await {
        Some(Ok(ConnStateEvt::SetLoc(mut answer, mut resp))) => {
            assert_eq!(&answer.to_vec().unwrap(), b"answer");
            resp.send(Ok(()));
        }
        _ => panic!("unexpected"),
    };

    println!("set loc");

    match sig_evt.recv().await {
        Some(Ok(SigStateEvt::SndAnswer(id, mut buf, mut resp))) => {
            assert_eq!(id, id_b);
            assert_eq!(&buf.to_vec().unwrap(), b"answer");
            resp.send(Ok(()));
        }
        _ => panic!("unexpected"),
    }

    println!("sent answer");

    // TODO - delete me
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;

    // -- cleanup -- //

    state.close(Error::id("TestShutdown"));

    assert!(matches!(
        state_evt.recv().await,
        Some(Err(err)) if &err.to_string() == "TestShutdown",
    ));

    // erm... is this what we want??
    assert!(matches!(
        state_evt.recv().await,
        Some(Err(err)) if &err.to_string() == "Dropped",
    ));

    assert!(matches!(state_evt.recv().await, None));
}
