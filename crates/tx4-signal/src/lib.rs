#![deny(missing_docs)]
#![deny(warnings)]
#![deny(unsafe_code)]

//! Holochain webrtc signal server / client.
//!
//! [![Project](https://img.shields.io/badge/project-holochain-blue.svg?style=flat-square)](http://holochain.org/)
//! [![Forum](https://img.shields.io/badge/chat-forum%2eholochain%2enet-blue.svg?style=flat-square)](https://forum.holochain.org)
//! [![Chat](https://img.shields.io/badge/chat-chat%2eholochain%2enet-blue.svg?style=flat-square)](https://chat.holochain.org)
//!
//! [![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://opensource.org/licenses/MIT)
//! [![License: Apache-2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://www.apache.org/licenses/LICENSE-2.0)
//!

/// Re-exported dependencies.
pub mod deps {
    pub use tx4_core::deps::*;
}

use deps::*;

pub use tx4_core::{Error, ErrorExt, Id, Result};

use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;

#[allow(dead_code)]
pub(crate) static WS_CONFIG: WebSocketConfig = WebSocketConfig {
    max_send_queue: Some(tx4_core::ws::MAX_SEND_QUEUE),
    max_message_size: Some(tx4_core::ws::MAX_MESSAGE_SIZE),
    max_frame_size: Some(tx4_core::ws::MAX_FRAME_SIZE),
    accept_unmasked_frames: false,
};

pub(crate) fn tcp_configure(
    socket: tokio::net::TcpStream,
) -> Result<tokio::net::TcpStream> {
    let socket = socket.into_std()?;
    let socket = socket2::Socket::from(socket);

    let keepalive = socket2::TcpKeepalive::new()
        .with_time(std::time::Duration::from_secs(7))
        .with_interval(std::time::Duration::from_secs(7));

    // we'll close unresponsive connections after 21-28 seconds (7 * 3)
    // (it's a little unclear how long it'll wait after the final probe)
    #[cfg(any(target_os = "linux", target_vendor = "apple"))]
    let keepalive = keepalive.with_retries(3);

    socket.set_tcp_keepalive(&keepalive)?;

    let socket = std::net::TcpStream::from(socket);
    tokio::net::TcpStream::from_std(socket)
}

mod cli;
pub use cli::*;

//pub mod srv;
pub mod tls;
//pub mod util;

/*
#[cfg(test)]
mod tests {
    use super::*;
    use lair_keystore_api::prelude::*;
    use std::sync::Arc;

    const ICE_SERVERS: &str = r#"[
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
    ]"#;

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
            Cb: FnMut(cli::SigMessage) + 'static + Send,
        {
            let passphrase = sodoken::BufRead::new_no_lock(b"test-passphrase");
            let keystore_config = PwHashLimits::Minimum
                .with_exec(|| {
                    LairServerConfigInner::new("/", passphrase.clone())
                })
                .await
                .unwrap();

            let keystore = PwHashLimits::Minimum
                .with_exec(|| {
                    lair_keystore_api::in_proc_keystore::InProcKeystore::new(
                        Arc::new(keystore_config),
                        lair_keystore_api::mem_store::create_mem_store_factory(
                        ),
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

            let tls = tls::TlsConfigBuilder::default()
                .with_danger_no_server_verify(true)
                .build()
                .unwrap();

            let cli = cli::Cli::builder()
                .with_tls(tls)
                .with_lair_client(lair_client)
                .with_lair_tag(tag)
                .with_recv_cb(recv_cb)
                .with_url(
                    url::Url::parse(&format!("wss://localhost:{}", port))
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
    async fn sanity() {
        init_tracing();

        let ice: serde_json::Value = serde_json::from_str(ICE_SERVERS).unwrap();
        let ice = serde_json::to_string(&ice).unwrap();

        let (cert, key) = tls::tls_self_signed().unwrap();
        let tls = tls::TlsConfigBuilder::default()
            .with_cert(cert, key)
            .build()
            .unwrap();
        let srv = srv::Srv::builder()
            .with_port(0)
            .with_tls(tls)
            .with_ice_servers(ice)
            .with_allow_demo(true)
            .build()
            .await
            .unwrap();
        let bound_port = srv.bound_port();
        tracing::info!(?bound_port);

        #[derive(Debug)]
        enum In {
            Cli1(cli::SigMessage),
            Cli2(cli::SigMessage),
        }

        let (in_send, mut in_recv) = tokio::sync::mpsc::unbounded_channel();

        let mut cli1 = {
            let in_send = in_send.clone();
            Test::new(bound_port, move |msg| {
                in_send.send(In::Cli1(msg)).unwrap();
            })
            .await
        };
        let cli1_addr = cli1.cli.local_addr().clone();
        tracing::warn!(%cli1_addr);
        let cli1_sig_id = signal_id_from_addr(&cli1_addr).unwrap();
        let cli1_pk = pk_from_addr(&cli1_addr).unwrap();
        tracing::info!(?cli1_sig_id, ?cli1_pk);

        let mut cli2 = Test::new(bound_port, move |msg| {
            in_send.send(In::Cli2(msg)).unwrap();
        })
        .await;
        let cli2_addr = cli2.cli.local_addr().clone();
        let cli2_sig_id = signal_id_from_addr(&cli2_addr).unwrap();
        let cli2_pk = pk_from_addr(&cli2_addr).unwrap();
        tracing::info!(?cli2_sig_id, ?cli2_pk);

        cli1.cli
            .offer(
                &cli2_sig_id,
                &cli2_pk,
                &serde_json::json!({ "type": "offer" }),
            )
            .await
            .unwrap();

        let msg = in_recv.recv().await;
        tracing::info!(?msg);
        assert!(matches!(msg, Some(In::Cli2(cli::SigMessage::Offer { .. }))));

        cli2.cli
            .answer(
                &cli1_sig_id,
                &cli1_pk,
                &serde_json::json!({ "type": "answer" }),
            )
            .await
            .unwrap();

        let msg = in_recv.recv().await;
        tracing::info!(?msg);
        assert!(matches!(
            msg,
            Some(In::Cli1(cli::SigMessage::Answer { .. }))
        ));

        cli1.cli
            .ice(
                &cli2_sig_id,
                &cli2_pk,
                &serde_json::json!({ "type": "ice" }),
            )
            .await
            .unwrap();

        let msg = in_recv.recv().await;
        tracing::info!(?msg);
        assert!(matches!(msg, Some(In::Cli2(cli::SigMessage::ICE { .. }))));

        cli1.cli.demo().await.unwrap();

        for _ in 0..2 {
            let msg = in_recv.recv().await;
            tracing::info!(?msg);
            let inner = match msg {
                Some(In::Cli1(m)) => m,
                Some(In::Cli2(m)) => m,
                _ => panic!("unexpected eos"),
            };
            assert!(matches!(inner, cli::SigMessage::Demo { .. }));
        }
    }
}
*/
