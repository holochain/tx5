use crate::*;
use lair_keystore_api::prelude::*;
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

struct Test {
    pub _keystore: lair_keystore_api::in_proc_keystore::InProcKeystore,
    pub cli: cli::Cli,
}

impl Test {
    pub async fn new<Cb>(port: u16, recv_cb: Cb) -> Self
    where
        Cb: FnMut(SignalMsg) + 'static + Send,
    {
        let passphrase = sodoken::BufRead::new_no_lock(b"test-passphrase");
        let keystore_config = PwHashLimits::Minimum
            .with_exec(|| LairServerConfigInner::new("/", passphrase.clone()))
            .await
            .unwrap();

        let keystore = PwHashLimits::Minimum
            .with_exec(|| {
                lair_keystore_api::in_proc_keystore::InProcKeystore::new(
                    Arc::new(keystore_config),
                    lair_keystore_api::mem_store::create_mem_store_factory(),
                    passphrase,
                )
            })
            .await
            .unwrap();

        let lair_client = keystore.new_client().await.unwrap();
        let tag: Arc<str> =
            rand_utf8::rand_utf8(&mut rand::thread_rng(), 32).into();

        lair_client
            .new_seed(tag.clone(), None, false)
            .await
            .unwrap();

        let cli = cli::Cli::builder()
            .with_lair_client(lair_client)
            .with_lair_tag(tag)
            .with_recv_cb(recv_cb)
            .with_url(
                url::Url::parse(&format!("ws://localhost:{}/tx5-ws", port))
                    .unwrap(),
            )
            .build()
            .await
            .unwrap();

        Self {
            _keystore: keystore,
            cli,
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn wrong_version() {
    init_tracing();

    let mut srv_config = tx5_signal_srv::Config::default();
    srv_config.port = 0;
    srv_config.ice_servers = serde_json::json!([]);
    srv_config.demo = true;

    let (srv_driver, addr_list, _) =
        tx5_signal_srv::exec_tx5_signal_srv(srv_config).unwrap();

    let srv_port = addr_list.get(0).unwrap().port();

    tracing::info!(%srv_port);

    tokio::select! {
        _ = srv_driver => (),
        _ = async move {
            // TODO remove
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;

            tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{srv_port}/tx5-ws/v1/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA")).await.unwrap();
            assert!(tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{srv_port}/tx5-ws/v0/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA")).await.is_err());
        } => (),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn ping() {
    init_tracing();

    let mut srv_config = tx5_signal_srv::Config::default();
    srv_config.port = 0;
    srv_config.ice_servers = serde_json::json!([]);
    srv_config.demo = true;

    let (srv_driver, addr_list, _) =
        tx5_signal_srv::exec_tx5_signal_srv(srv_config).unwrap();

    let srv_port = addr_list.get(0).unwrap().port();

    tokio::select! {
        _ = srv_driver => (),
        _ = async move {
            // TODO remove
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;

            ping_inner(srv_port).await;
        } => (),
    }
}

async fn ping_inner(srv_port: u16) {
    let cli1 = cli::Cli::builder()
        .with_recv_cb(|_| {})
        .with_url(
            url::Url::parse(&format!("ws://localhost:{}/tx5-ws", srv_port))
                .unwrap(),
        )
        .build()
        .await
        .unwrap();

    let cli2 = cli::Cli::builder()
        .with_recv_cb(|_| {})
        .with_url(
            url::Url::parse(&format!("ws://localhost:{}/tx5-ws", srv_port))
                .unwrap(),
        )
        .build()
        .await
        .unwrap();

    println!("cli1, ping2: {}", cli1.ping(cli2.local_id()).await.unwrap());
    println!("cli2, ping1: {}", cli2.ping(cli1.local_id()).await.unwrap());
}

#[tokio::test(flavor = "multi_thread")]
async fn sanity() {
    init_tracing();

    let mut srv_config = tx5_signal_srv::Config::default();
    srv_config.port = 0;
    srv_config.ice_servers = serde_json::json!([]);
    srv_config.demo = true;

    let (srv_driver, addr_list, _) =
        tx5_signal_srv::exec_tx5_signal_srv(srv_config).unwrap();

    let srv_port = addr_list.get(0).unwrap().port();

    tracing::info!(%srv_port);

    tokio::select! {
        _ = srv_driver => (),
        _ = async move {
            // TODO remove
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;

            sanity_inner(srv_port).await;
        } => (),
    }
}

async fn sanity_inner(srv_port: u16) {
    #[derive(Debug)]
    enum In {
        Cli1(SignalMsg),
        Cli2(SignalMsg),
    }

    let (in_send, mut in_recv) = tokio::sync::mpsc::unbounded_channel();

    let cli1 = {
        let in_send = in_send.clone();
        Test::new(srv_port, move |msg| {
            in_send.send(In::Cli1(msg)).unwrap();
        })
        .await
    };

    let cli1_pk = cli1.cli.local_id();
    tracing::info!(%cli1_pk);

    let cli2 = Test::new(srv_port, move |msg| {
        in_send.send(In::Cli2(msg)).unwrap();
    })
    .await;

    let cli2_pk = cli2.cli.local_id();
    tracing::info!(%cli2_pk);

    cli1.cli
        .offer(cli2_pk, serde_json::json!({ "type": "offer" }))
        .await
        .unwrap();

    let msg = in_recv.recv().await;
    tracing::info!(?msg);
    assert!(matches!(msg, Some(In::Cli2(SignalMsg::Offer { .. }))));

    cli2.cli
        .answer(cli1_pk, serde_json::json!({ "type": "answer" }))
        .await
        .unwrap();

    let msg = in_recv.recv().await;
    tracing::info!(?msg);
    assert!(matches!(msg, Some(In::Cli1(SignalMsg::Answer { .. }))));

    cli1.cli
        .ice(cli2_pk, serde_json::json!({ "type": "ice" }))
        .await
        .unwrap();

    let msg = in_recv.recv().await;
    tracing::info!(?msg);
    assert!(matches!(msg, Some(In::Cli2(SignalMsg::Ice { .. }))));

    cli1.cli.demo();

    for _ in 0..2 {
        let msg = in_recv.recv().await;
        tracing::info!(?msg);
        let inner = match msg {
            Some(In::Cli1(m)) => m,
            Some(In::Cli2(m)) => m,
            _ => panic!("unexpected eos"),
        };
        assert!(matches!(inner, SignalMsg::Demo { .. }));
    }
}
