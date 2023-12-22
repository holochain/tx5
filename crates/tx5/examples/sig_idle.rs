//! Opens a connection to a running signal server, and sits idle.
//! This is useful to test our keepalive logic.

use std::sync::Arc;
use tx5::*;

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

    let args = std::env::args().collect::<Vec<_>>();
    let sig_url = args.get(1).expect("required signal_url");
    let sig_url = Tx5Url::new(sig_url).unwrap();
    println!("{sig_url}");

    let (ep, _ep_rcv) = Ep3::new(Arc::new(Config3::default())).await;

    ep.listen(sig_url).unwrap();

    let (_s, r) = tokio::sync::oneshot::channel::<()>();
    r.await.unwrap();
}
