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
    pub async fn new(
        port: u16,
    ) -> (Self, tokio::sync::mpsc::Receiver<SignalMsg>) {
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

        let (cli, msg_recv) = cli::Cli::builder()
            .with_lair_client(lair_client)
            .with_lair_tag(tag)
            .with_url(
                url::Url::parse(&format!("ws://localhost:{}/tx5-ws", port))
                    .unwrap(),
            )
            .build()
            .await
            .unwrap();

        (
            Self {
                _keystore: keystore,
                cli,
            },
            msg_recv,
        )
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn server_stop_restart() {
    init_tracing();

    let mut task = None;
    let mut port = None;

    for p in 31181..31191 {
        let mut srv_config = tx5_signal_srv::Config::default();
        srv_config.port = p;
        srv_config.ice_servers = serde_json::json!([]);

        if let Ok((srv_hnd, _, _)) =
            tx5_signal_srv::exec_tx5_signal_srv(srv_config)
        {
            task = Some(srv_hnd);
            port = Some(p);
            break;
        }
    }

    let mut task = task.unwrap();
    let port = port.unwrap();

    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // test setup
    let (cli1, mut rcv1) = Test::new(port).await;
    let id1 = *cli1.cli.local_id();

    cli1.cli
        .ice(id1, serde_json::json!({"test": "ice"}))
        .await
        .unwrap();

    let msg = rcv1.recv().await;
    tracing::info!(?msg);
    assert!(matches!(msg, Some(SignalMsg::Ice { .. })));

    // drop server
    drop(task);

    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // make sure it now errors

    // first just trigger an update
    let _ = cli1.cli.ice(id1, serde_json::json!({"test": "ice"})).await;

    // now our receive ends
    let msg = rcv1.recv().await;
    tracing::info!(?msg);
    assert!(matches!(msg, None));

    // now we get errors on send
    cli1.cli
        .ice(id1, serde_json::json!({"test": "ice"}))
        .await
        .unwrap_err();

    // new server on same port

    let mut srv_config = tx5_signal_srv::Config::default();
    srv_config.port = port;
    srv_config.ice_servers = serde_json::json!([]);

    let (srv_hnd, _, _) =
        tx5_signal_srv::exec_tx5_signal_srv(srv_config).unwrap();

    task = srv_hnd;

    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // test continuation
    let (cli1, mut rcv1) = Test::new(port).await;
    let id1 = *cli1.cli.local_id();

    cli1.cli
        .ice(id1, serde_json::json!({"test": "ice"}))
        .await
        .unwrap();

    let msg = rcv1.recv().await;
    tracing::info!(?msg);
    assert!(matches!(msg, Some(SignalMsg::Ice { .. })));

    // cleanup
    drop(task);
}

#[tokio::test(flavor = "multi_thread")]
async fn wrong_version() {
    init_tracing();

    let mut srv_config = tx5_signal_srv::Config::default();
    srv_config.port = 0;
    srv_config.ice_servers = serde_json::json!([]);
    srv_config.demo = true;

    let (_srv_hnd, addr_list, _) =
        tx5_signal_srv::exec_tx5_signal_srv(srv_config).unwrap();

    let srv_port = addr_list.get(0).unwrap().port();

    tracing::info!(%srv_port);

    // TODO remove
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;

    tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{srv_port}/tx5-ws/v1/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA")).await.unwrap();
    assert!(tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{srv_port}/tx5-ws/v0/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA")).await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn sanity() {
    init_tracing();

    let mut srv_config = tx5_signal_srv::Config::default();
    srv_config.port = 0;
    srv_config.ice_servers = serde_json::json!([]);
    srv_config.demo = true;

    let (_srv_hnd, addr_list, _) =
        tx5_signal_srv::exec_tx5_signal_srv(srv_config).unwrap();

    let srv_port = addr_list.get(0).unwrap().port();

    tracing::info!(%srv_port);

    // TODO remove
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;

    sanity_inner(srv_port).await;
}

async fn sanity_inner(srv_port: u16) {
    let (cli1, mut rcv1) = Test::new(srv_port).await;

    let cli1_pk = *cli1.cli.local_id();
    tracing::info!(%cli1_pk);

    let (cli2, mut rcv2) = Test::new(srv_port).await;

    let cli2_pk = *cli2.cli.local_id();
    tracing::info!(%cli2_pk);

    cli1.cli
        .offer(cli2_pk, serde_json::json!({ "type": "offer" }))
        .await
        .unwrap();

    let msg = rcv2.recv().await;
    tracing::info!(?msg);
    assert!(matches!(msg, Some(SignalMsg::Offer { .. })));

    cli2.cli
        .answer(cli1_pk, serde_json::json!({ "type": "answer" }))
        .await
        .unwrap();

    let msg = rcv1.recv().await;
    tracing::info!(?msg);
    assert!(matches!(msg, Some(SignalMsg::Answer { .. })));

    cli1.cli
        .ice(cli2_pk, serde_json::json!({ "type": "ice" }))
        .await
        .unwrap();

    let msg = rcv2.recv().await;
    tracing::info!(?msg);
    assert!(matches!(msg, Some(SignalMsg::Ice { .. })));

    cli1.cli.demo();

    let msg = rcv1.recv().await;
    tracing::info!(?msg);
    assert!(matches!(msg, Some(SignalMsg::Demo { .. })));

    let msg = rcv2.recv().await;
    tracing::info!(?msg);
    assert!(matches!(msg, Some(SignalMsg::Demo { .. })));
}
