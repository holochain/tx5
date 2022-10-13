use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn state_sanity() {
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

    let (sig_state, mut sig_evt) =
        sig_seed.result_ok(cli_a.clone()).await.unwrap();

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

    let (_conn_state, mut conn_evt) = conn_seed.result_ok().await.unwrap();

    println!("respondend naotehunadc");

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

    task.await.unwrap();

    // -- cleanup -- //

    state.close(Error::id("TestShutdown"));

    assert!(matches!(
        state_evt.recv().await,
        Some(Err(err)) if &err.to_string() == "TestShutdown",
    ));

    assert!(matches!(state_evt.recv().await, None));
}
