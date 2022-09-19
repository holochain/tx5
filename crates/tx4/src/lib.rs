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
    }
}
