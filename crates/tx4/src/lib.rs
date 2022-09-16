#![deny(missing_docs)]
#![deny(warnings)]
#![deny(unsafe_code)]

//! Tx4 - The main holochain tx4 webrtc networking crate.

#[cfg(any(
    not(any(feature = "backend-go-pion", feature = "backend-webrtc-rs")),
    all(feature = "backend-go-pion", feature = "backend-webrtc-rs"),
))]
compile_error!("Must specify exactly 1 webrtc backend");

/// Re-exported dependencies.
pub mod deps {
    pub use tx4_core;
    pub use tx4_core::deps::*;
}

use deps::serde;

pub use tx4_core::{Error, ErrorExt, Result};

mod buf;
pub use buf::*;

mod chan;
pub use chan::*;

mod conn;
pub use conn::*;

#[cfg(test)]
mod tests {
    use super::*;

    const STUN: &str = r#"{
    "iceServers": [
        {
          "urls": ["stun:openrelay.metered.ca:80"]
        },
        {
          "urls": ["turn:openrelay.metered.ca:80"],
          "username": "openrelayproject",
          "credential": "openrelayproject"
        },
        {
          "urls": ["turn:openrelay.metered.ca:443"],
          "username": "openrelayproject",
          "credential": "openrelayproject"
        },
        {
          "urls": ["turn:openrelay.metered.ca:443?transport=tcp"],
          "username": "openrelayproject",
          "credential": "openrelayproject"
        }
    ]
}"#;

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

    #[tokio::test(flavor = "multi_thread")]
    async fn happy_path() {
        init_tracing();

        let mut conn1 =
            PeerConnection::new(Buf::from_slice(STUN).unwrap(), move |_evt| {})
                .await
                .unwrap();

        let _offer = conn1.create_offer(OfferConfig::default()).await.unwrap();
    }
}
