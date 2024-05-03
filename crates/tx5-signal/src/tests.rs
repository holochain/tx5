use crate::*;
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

pub struct TestSrv {
    server: sbd_server::SbdServer,
}

impl TestSrv {
    pub async fn new() -> Self {
        let config = Arc::new(sbd_server::Config {
            bind: vec!["127.0.0.1:0".to_string(), "[::1]:0".to_string()],
            ..Default::default()
        });

        let server = sbd_server::SbdServer::new(config).await.unwrap();

        Self { server }
    }

    pub async fn cli(&self) -> (SignalConnection, MsgRecv) {
        for addr in self.server.bind_addrs() {
            if let Ok(r) = SignalConnection::connect(
                &format!("ws://{addr}"),
                Arc::new(SignalConfig {
                    listener: true,
                    allow_plain_text: true,
                    ..Default::default()
                }),
            )
            .await
            {
                return r;
            }
        }

        panic!()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn sanity() {
    init_tracing();

    let test = TestSrv::new().await;

    let (cli1, mut rcv1) = test.cli().await;

    let cli1_pk = cli1.pub_key().clone();
    tracing::info!(?cli1_pk);

    let (cli2, mut rcv2) = test.cli().await;

    let cli2_pk = cli2.pub_key().clone();
    tracing::info!(?cli2_pk);

    cli1.send_offer(&cli2_pk, serde_json::json!({ "type": "offer" }))
        .await
        .unwrap();

    let (_, msg) = rcv2.recv_message().await.unwrap();
    tracing::info!(?msg);
    assert!(matches!(msg, SignalMessage::Offer { .. }));

    cli2.send_answer(&cli1_pk, serde_json::json!({ "type": "answer" }))
        .await
        .unwrap();

    let (_, msg) = rcv1.recv_message().await.unwrap();
    tracing::info!(?msg);
    assert!(matches!(msg, SignalMessage::Answer { .. }));

    cli1.send_ice(&cli2_pk, serde_json::json!({ "type": "ice" }))
        .await
        .unwrap();

    let (_, msg) = rcv2.recv_message().await.unwrap();
    tracing::info!(?msg);
    assert!(matches!(msg, SignalMessage::Ice { .. }));
}
