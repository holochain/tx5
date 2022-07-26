#![deny(warnings)]
#![deny(unsafe_code)]

use std::sync::Arc;

type Result<T> = std::result::Result<T, std::io::Error>;

/// Tx3 helper until `std::io::Error::other()` is stablized
pub fn other_err<E: Into<Box<dyn std::error::Error + Send + Sync>>>(error: E) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Other, error)
}

pub fn debug_spawn<T>(future: T) -> tokio::task::JoinHandle<()>
where
    T: std::future::Future<Output = Result<()>> + Send + 'static,
{
    tokio::task::spawn(async move {
        if let Err(err) = future.await {
            tracing::debug!(?err);
        }
    })
}

mod config;

mod lair;

mod state;

mod con_;

mod core;

mod sig;

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let subscriber = tracing_subscriber::FmtSubscriber::builder()
        .with_env_filter(
            tracing_subscriber::filter::EnvFilter::builder()
                .with_default_directive(tracing_subscriber::filter::LevelFilter::INFO.into())
                .from_env_lossy(),
        )
        .with_file(true)
        .with_line_number(true)
        .finish();
    tracing::subscriber::set_global_default(subscriber).unwrap();

    if let Err(err) = main_err().await {
        eprintln!("{}", err);
        std::process::exit(1);
    }
}

async fn main_err() -> Result<()> {
    let config = config::load_config().await?;
    tracing::info!(?config, "loaded config");

    let lair = lair::load(config.clone()).await?;

    let core = core::Core::new(config.friendly_name.clone(), config.shoutout.clone());

    sig::Sig::spawn_to_core(config.clone(), lair, core).await?;

    struct Pend;

    impl std::future::Future for Pend {
        type Output = ();

        fn poll(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Self::Output> {
            std::task::Poll::Pending
        }
    }

    Pend.await;

    Ok(())
}
