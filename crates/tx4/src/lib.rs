#![deny(missing_docs)]
#![deny(warnings)]
#![deny(unsafe_code)]

//! Tx4 - The main holochain tx4 webrtc networking crate.
//!
//! [![Project](https://img.shields.io/badge/project-holochain-blue.svg?style=flat-square)](http://holochain.org/)
//! [![Forum](https://img.shields.io/badge/chat-forum%2eholochain%2enet-blue.svg?style=flat-square)](https://forum.holochain.org)
//! [![Chat](https://img.shields.io/badge/chat-chat%2eholochain%2enet-blue.svg?style=flat-square)](https://chat.holochain.org)
//!
//! [![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://opensource.org/licenses/MIT)
//! [![License: Apache-2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://www.apache.org/licenses/LICENSE-2.0)
//!
//! # WebRTC Backend Features
//!
//! Tx4 can be backed currently by 1 of 2 backend webrtc libraries.
//!
//! - <b><i>`*`DEFAULT`*`</i></b> `backend-go-pion` - The pion webrtc library
//!   writen in go (golang).
//!   - [https://github.com/pion/webrtc](https://github.com/pion/webrtc)
//! - `backend-webrtc-rs` - The rust webrtc library.
//!   - [https://github.com/webrtc-rs/webrtc](https://github.com/webrtc-rs/webrtc)
//!
//! The go pion library is currently the default as it is more mature
//! and well tested, but comes with some overhead of calling into a different
//! memory/runtime. When the rust library is stable enough for holochain's
//! needs, we will switch the default. To switch now, or if you want to
//! make sure the backend doesn't change out from under you, set
//! no-default-features and explicitly enable the backend of your choice.

#[cfg(any(
    not(any(feature = "backend-go-pion", feature = "backend-webrtc-rs")),
    all(feature = "backend-go-pion", feature = "backend-webrtc-rs"),
))]
compile_error!("Must specify exactly 1 webrtc backend");

/// Re-exported dependencies.
pub mod deps {
    pub use tx4_core;
    pub use tx4_core::deps::*;
    pub use tx4_signal;
    pub use tx4_signal::deps::*;
}

use deps::serde;

pub use tx4_core::{Error, ErrorExt, Result};
pub use tx4_signal::{Cli, CliBuilder};

// TODO un-pub
pub mod actor;
//use actor::*;

mod buf;
pub use buf::*;

mod chan;
pub use chan::*;

mod conn;
pub use conn::*;

// TODO un-pub
pub mod state;

mod endpoint;
pub use endpoint::*;

#[cfg(test)]
mod tests {
    use super::*;
    use deps::*;
    use lair_keystore_api::prelude::*;
    use std::sync::Arc;

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

    pub struct TestSignal {
        addr: std::net::SocketAddr,
        abort: tokio::task::JoinHandle<()>,
    }

    impl Drop for TestSignal {
        fn drop(&mut self) {
            self.abort.abort();
        }
    }

    impl TestSignal {
        pub async fn new() -> Self {
            let mut config = tx4_signal_srv::Config::default();
            config.port = 0;
            config.ice_servers = serde_json::from_str(STUN).unwrap();
            config.demo = true;
            let (addr, driver) =
                tx4_signal_srv::exec_tx4_signal_srv(config).unwrap();

            let abort = tokio::task::spawn(driver);

            Self { addr, abort }
        }

        pub fn local_addr(&self) -> &std::net::SocketAddr {
            &self.addr
        }
    }

    pub struct TestLair {
        lair_client: lair_keystore_api::LairClient,
        tag: Arc<str>,
    }

    impl TestLair {
        pub async fn new() -> Self {
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

            Self { lair_client, tag }
        }

        pub fn lair_client(&self) -> &LairClient {
            &self.lair_client
        }

        pub fn tag(&self) -> &Arc<str> {
            &self.tag
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn happy_path() {
        init_tracing();

        let srv = TestSignal::new().await;
        let srv_addr = url::Url::parse(&format!(
            "ws://localhost:{}",
            srv.local_addr().port()
        ))
        .unwrap();

        let lair1 = TestLair::new().await;

        let ep1 = Endpoint::new(
            lair1.lair_client().clone(),
            lair1.tag().clone(),
            |evt| {
                tracing::debug!("EP1:EVT:{:?}", evt);
            },
        )
        .unwrap();
        ep1.listen(srv_addr.clone()).await.unwrap();
        let addr1 = ep1.url_list().await.unwrap().remove(0);
        println!("addr1: {}", addr1);

        let lair2 = TestLair::new().await;

        let ep2 = Endpoint::new(
            lair2.lair_client().clone(),
            lair2.tag().clone(),
            |evt| {
                tracing::debug!("EP2:EVT:{:?}", evt);
            },
        )
        .unwrap();
        ep2.listen(srv_addr).await.unwrap();
        let addr2 = ep2.url_list().await.unwrap().remove(0);
        println!("addr2: {}", addr2);

        //ep1.connect(addr2).await.unwrap();

        /*
        let (conn_send, mut conn_recv) = tokio::sync::mpsc::unbounded_channel();

        let conn_send_1 = conn_send.clone();
        let mut conn1 =
            PeerConnection::new(Buf::from_slice(STUN).unwrap(), move |evt| {
                let _ = conn_send_1.send((1, evt));
            })
            .await
            .unwrap();

        let (chan_send, mut chan_recv) = tokio::sync::mpsc::unbounded_channel();

        let chan1 = conn1
            .create_data_channel(
                DataChannelConfig::default().with_label("data"),
            )
            .await
            .unwrap();
        let chan_send_1 = chan_send.clone();
        let chan1_fut = chan1.handle(move |evt| {
            let _ = chan_send_1.send((1, evt));
        });

        let mut offer =
            conn1.create_offer(OfferConfig::default()).await.unwrap();
        conn1.set_local_description(&mut offer).await.unwrap();

        let conn_send_2 = conn_send;
        let mut conn2 =
            PeerConnection::new(Buf::from_slice(STUN).unwrap(), move |evt| {
                let _ = conn_send_2.send((2, evt)).is_err();
            })
            .await
            .unwrap();

        conn2.set_remote_description(offer).await.unwrap();
        let mut answer =
            conn2.create_answer(AnswerConfig::default()).await.unwrap();
        conn2.set_local_description(&mut answer).await.unwrap();
        conn1.set_remote_description(answer).await.unwrap();

        let mut chan2 = None;
        while let Some((id, evt)) = conn_recv.recv().await {
            match (id, evt) {
                (1, PeerConnectionEvent::IceCandidate(buf)) => {
                    conn2.add_ice_candidate(buf).await.unwrap();
                }
                (2, PeerConnectionEvent::IceCandidate(buf)) => {
                    conn1.add_ice_candidate(buf).await.unwrap();
                }
                (2, PeerConnectionEvent::DataChannel(ch)) => {
                    chan2 = Some(ch);
                    break;
                }
                _ => unreachable!(),
            }
        }
        let chan2 = chan2.unwrap();
        let chan_send_2 = chan_send;
        let chan2_fut = chan2.handle(move |evt| {
            let _ = chan_send_2.send((2, evt));
        });

        let mut chan1 = chan1_fut.await.unwrap();
        let mut chan2 = chan2_fut.await.unwrap();

        chan1
            .send(Buf::from_slice(b"hello").unwrap())
            .await
            .unwrap();

        let (id, res) = chan_recv.recv().await.unwrap();
        assert_eq!(2, id);
        let mut res = match res {
            DataChannelEvent::Message(buf) => buf,
            _ => unreachable!(),
        };
        assert_eq!(b"hello", res.to_vec().unwrap().as_slice());

        chan2
            .send(Buf::from_slice(b"world").unwrap())
            .await
            .unwrap();

        let (id, res) = chan_recv.recv().await.unwrap();
        assert_eq!(1, id);
        let mut res = match res {
            DataChannelEvent::Message(buf) => buf,
            _ => unreachable!(),
        };
        assert_eq!(b"world", res.to_vec().unwrap().as_slice());
        */
    }
}
