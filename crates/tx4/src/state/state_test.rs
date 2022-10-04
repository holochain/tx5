use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn state_sanity() {
    let (state, mut state_evt) = State::new();

    let sig_a: Tx4Url = Tx4Url::new(
        "wss://a/tx4-ws/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
    )
    .unwrap();

    let task = {
        let state = state.clone();
        // can't do this inline, since it won't resolve until result_ok
        // call on the seed below
        tokio::task::spawn(async move {
            state.listener_sig(sig_a.clone()).await.unwrap()
        })
    };

    let sig_seed = match state_evt.recv().await {
        Some(Ok(StateEvt::NewSig(seed))) => seed,
        _ => panic!("unexpected"),
    };

    let (_sig_state, _sig_evt) = sig_seed.result_ok();

    task.await.unwrap();

    state.shutdown(Some(Error::id("TestShutdown")));
}
