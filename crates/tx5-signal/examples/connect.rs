//! Connect to a signal server, fetch and print the ice servers, and disconnect.

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

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    init_tracing();

    let sig_url = std::env::args().collect::<Vec<_>>();
    let sig_url = sig_url.get(1).expect("expect sig_url");

    tracing::info!(%sig_url);

    let config = tx5_signal::SignalConfig {
        client_config: tx5_signal::SbdClientConfig {
            allow_plain_text: true,
            ..Default::default()
        },
        ..Default::default()
    };

    let (con, _recv) = tx5_signal::SignalConnection::connect(
        &sig_url,
        std::sync::Arc::new(config),
    )
    .await
    .unwrap();

    println!("Connected at: {sig_url}/{:?}", con.pub_key());
}
