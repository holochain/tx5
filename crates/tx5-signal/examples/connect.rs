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

    /*
    let (cli, _rcv) = tx5_signal::Cli::builder()
        .with_url(sig_url)
        .build()
        .await
        .expect("expect can build tx5_signal::Cli");

    println!(
        "{}",
        serde_json::to_string_pretty(&cli.ice_servers()).expect("expect json")
    );
    */
}
