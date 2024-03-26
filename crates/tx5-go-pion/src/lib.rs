#![deny(missing_docs)]
#![doc = tx5_core::__doc_header!()]
//! # tx5-go-pion
//!
//! Higher level rust bindings to the go pion webrtc library.

/// Re-exported dependencies.
pub mod deps {
    pub use libc;
    pub use once_cell;
    // uhhh... rust? this is a pub export...
    #[allow(unused_imports)]
    pub use tx5_core::deps::*;
    pub use tx5_go_pion_sys;
    pub use tx5_go_pion_sys::deps::*;
}

/// We need to keep all the intermediaries to ensure lifetimes.
macro_rules! r2id {
    ($n:ident) => {
        let mut $n = $n.into();
        let $n = $n.as_mut_ref()?;
        let $n = $n.0;
    };
}

pub use tx5_core::Tx5InitConfig;

#[allow(clippy::type_complexity)]
async fn tx5_init() -> std::result::Result<(), String> {
    static SHARED: once_cell::sync::Lazy<
        futures::future::Shared<
            std::pin::Pin<
                Box<
                    dyn std::future::Future<
                            Output = std::result::Result<(), String>,
                        >
                        + 'static
                        + Send,
                >,
            >,
        >,
    > = once_cell::sync::Lazy::new(|| {
        futures::FutureExt::shared(Box::pin(async move {
            let mut config = GoBufRef::json(Tx5InitConfig::get());
            let config = config.as_mut_ref().map_err(|e| format!("{e:?}"))?;
            let config = config.0;
            unsafe {
                tx5_go_pion_sys::API
                    .tx5_init(config)
                    .map_err(|e| format!("{e:?}"))?;
            }
            <std::result::Result<(), String>>::Ok(())
        }))
    });

    SHARED.clone().await
}

use deps::*;

pub use tx5_core::{Error, ErrorExt, Id, Result};

mod evt;
pub use evt::*;

mod go_buf;
pub use go_buf::*;

mod peer_con;
pub use peer_con::*;

mod data_chan;
pub use data_chan::*;

#[cfg(test)]
mod tests {
    use super::*;
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

    #[tokio::test(flavor = "multi_thread")]
    async fn peer_con() {
        init_tracing();

        let (ice, turn) = tx5_go_pion_turn::test_turn_server().await.unwrap();

        let config: PeerConnectionConfig =
            serde_json::from_str(&format!("{{\"iceServers\":[{ice}]}}"))
                .unwrap();

        let (peer1, mut prcv1) = PeerConnection::new(&config).await.unwrap();
        let (peer2, mut prcv2) = PeerConnection::new(&config).await.unwrap();

        let (data1, mut drcv1) = peer1
            .create_data_channel(DataChannelConfig {
                label: Some("data".into()),
            })
            .await
            .unwrap();

        let mut offer =
            peer1.create_offer(OfferConfig::default()).await.unwrap();
        peer1
            .set_local_description(offer.try_clone().unwrap())
            .await
            .unwrap();
        peer2.set_remote_description(offer).await.unwrap();
        let mut answer =
            peer2.create_answer(AnswerConfig::default()).await.unwrap();
        peer2
            .set_local_description(answer.try_clone().unwrap())
            .await
            .unwrap();
        peer1.set_remote_description(answer).await.unwrap();

        let (data2, mut drcv2) = loop {
            if let Some(evt) = prcv2.recv().await {
                match evt {
                    PeerConnectionEvent::Error(err) => panic!("{err:?}"),
                    PeerConnectionEvent::State(_) => (),
                    PeerConnectionEvent::ICECandidate(ice) => {
                        peer1.add_ice_candidate(ice).await.unwrap();
                    }
                    PeerConnectionEvent::DataChannel(data2, drcv2) => {
                        break (data2, drcv2);
                    }
                }
            } else {
                panic!("receiver ended");
            }
        };

        #[derive(Debug)]
        enum FinishState {
            Start,
            Msg1,
            Msg2,
            Done,
        }

        impl FinishState {
            fn is_done(&self) -> bool {
                matches!(self, Self::Done)
            }

            fn msg1(&self) -> Self {
                match self {
                    Self::Start => Self::Msg1,
                    Self::Msg1 => Self::Msg1,
                    Self::Msg2 => Self::Done,
                    oth => panic!("expected not Done, got: {oth:?}"),
                }
            }

            fn msg2(&self) -> Self {
                match self {
                    Self::Start => Self::Msg2,
                    Self::Msg1 => Self::Done,
                    oth => panic!("expected Start or Msg1, got: {oth:?}"),
                }
            }
        }

        let mut state = FinishState::Start;

        loop {
            tokio::select! {
                evt = prcv1.recv() => match evt {
                    Some(PeerConnectionEvent::State(_)) => (),
                    Some(PeerConnectionEvent::ICECandidate(ice)) => {
                        peer2.add_ice_candidate(ice).await.unwrap();
                    }
                    oth => panic!("unexpected: {oth:?}"),
                },
                evt = prcv2.recv() => match evt {
                    Some(PeerConnectionEvent::State(_)) => (),
                    Some(PeerConnectionEvent::ICECandidate(ice)) => {
                        peer1.add_ice_candidate(ice).await.unwrap();
                    }
                    oth => panic!("unexpected: {oth:?}"),
                },
                evt = drcv1.recv() => match evt {
                    Some(DataChannelEvent::BufferedAmountLow) => (),
                    Some(DataChannelEvent::Open) => {
                        assert_eq!(
                            "data",
                            &String::from_utf8_lossy(
                               &data2.label().unwrap().to_vec().unwrap()),
                        );
                        println!(
                            "data1 pre-send buffered amount: {}",
                            data1.set_buffered_amount_low_threshold(5).unwrap(),
                        );
                        println!(
                            "data1 post-send buffered amount: {}",
                            data1.send(GoBuf::from_slice(b"hello").unwrap()).await.unwrap(),
                        );
                    }
                    Some(DataChannelEvent::Message(mut buf)) => {
                        assert_eq!(
                            "world",
                            &String::from_utf8_lossy(&buf.to_vec().unwrap()),
                        );

                        state = state.msg1();
                    }
                    oth => panic!("unexpected: {oth:?}"),
                },
                evt = drcv2.recv() => match evt {
                    Some(DataChannelEvent::BufferedAmountLow) => (),
                    Some(DataChannelEvent::Open) => {
                        assert_eq!(
                            "data",
                            &String::from_utf8_lossy(
                               &data2.label().unwrap().to_vec().unwrap()),
                        );
                        println!(
                            "data2 pre-send buffered amount: {}",
                            data2.set_buffered_amount_low_threshold(5).unwrap(),
                        );
                        println!(
                            "data2 post-send buffered amount: {}",
                            data2.send(GoBuf::from_slice(b"world").unwrap()).await.unwrap(),
                        );
                    }
                    Some(DataChannelEvent::Message(mut buf)) => {
                        assert_eq!(
                            "hello",
                            &String::from_utf8_lossy(&buf.to_vec().unwrap()),
                        );

                        state = state.msg2();
                    }
                    oth => panic!("unexpected: {oth:?}"),
                },
            }
            if state.is_done() {
                println!(
                    "peer1: {}",
                    String::from_utf8_lossy(
                        &peer1.stats().await.unwrap().to_vec().unwrap()
                    ),
                );
                println!(
                    "peer2: {}",
                    String::from_utf8_lossy(
                        &peer1.stats().await.unwrap().to_vec().unwrap()
                    ),
                );
                break;
            }
        }

        let data1 = Arc::new(data1);

        let mut all = Vec::new();

        const COUNT: usize = 10;

        let bar = Arc::new(tokio::sync::Barrier::new(COUNT));

        for i in 0..COUNT {
            let hnd = tokio::runtime::Handle::current();
            let bar = bar.clone();
            let data1 = data1.clone();
            all.push(std::thread::spawn(move || {
                hnd.block_on(async move {
                    println!("send {i}");
                    bar.wait().await;
                    data1
                        .send(GoBuf::from_slice(b"hello").unwrap())
                        .await
                        .unwrap();
                    println!("sent {i}");
                });
            }));
        }

        let mut r_count = 0;
        while let Some(evt) = drcv2.recv().await {
            println!("{evt:?}");
            if matches!(evt, DataChannelEvent::Message(_)) {
                r_count += 1;
                println!("got {r_count}");
                if r_count >= COUNT {
                    break;
                }
            }
        }

        for (i, t) in all.into_iter().enumerate() {
            println!("await thread {i}");
            t.join().unwrap();
            println!("thread {i} complete");
        }

        println!("close data 1");
        data1.close(Error::id("").into());
        println!("close data 2");
        data2.close(Error::id("").into());
        println!("close peer 1");
        peer1.close(Error::id(""));
        println!("close peer 2");
        peer2.close(Error::id(""));

        println!("close turn");
        turn.stop().await.unwrap();

        println!("all done.");
    }
}
