use tx5_go_pion::*;
use crate::deps::serde_json;
use std::io::Result;

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    println!("RESULT: {:?}", check_nat_type(
        "stun:turn-1.infra.holochain.org:443".to_string(),
        "stun:turn-2.infra.holochain.org:443".to_string(),
    ).await);
}

#[derive(Debug)]
pub enum NatType {
    NormalNatStunOkay,
    SymmetricNatTurnRequired,
    UdpFailureTurnRequired,
}

pub async fn check_nat_type(stun_a: String, stun_b: String) -> NatType {
    match async move {
        let ice = IceServer {
            urls: vec![stun_a, stun_b],
            username: None,
            credential: None,
        };

        let config = PeerConnectionConfig {
            ice_servers: vec![ice],
        };

        println!("running with: {config:?}");

        let (con, mut rcv) = PeerConnection::new(config).await?;

        let (s, r) = tokio::sync::oneshot::channel();
        let mut s = Some(s);

        let result = std::sync::Arc::new(std::sync::Mutex::new(<std::collections::HashMap<String, Vec<String>>>::new()));
        let result2 = result.clone();

        tokio::task::spawn(async move {
            while let Some(evt) = rcv.recv().await {
                match evt {
                    PeerConnectionEvent::ICEGatheringState(PeerConnectionICEGatheringState::Complete) => {
                        if let Some(s) = s.take() {
                            let _ = s.send(());
                        }
                    }
                    PeerConnectionEvent::ICECandidate(mut ice) => {
                        let ice: serde_json::Value = ice.as_json().unwrap();
                        if let Some(ice) = ice.as_object() {
                            if let Some(ice) = ice.get("candidate") {
                                if let Some(ice) = ice.as_str() {
                                    if ice.contains("srflx") {
                                        let mut parts = if ice.starts_with("a=candidate:") {
                                            ice.split("a=candidate:")
                                        } else {
                                            ice.split("candidate:")
                                        };
                                        parts.next().unwrap();
                                        let parts = parts.next().unwrap().split(" ").collect::<Vec<_>>();

                                        //println!("{parts:?}");

                                        let mut mode = 0;
                                        for v in parts.iter() {
                                            if mode == 0 && v == &"rport" {
                                                mode = 1
                                            } else if mode == 1 {
                                                mode = 0;
                                                result.lock().unwrap().entry(v.to_string()).or_default().push(parts.get(5).unwrap().to_string());
                                            }
                                        }
                                    }
                                }
                            } else {
                                println!("done");
                            }
                        }
                    }
                    _evt => {
                        //println!("other event: {evt:?}");
                    }
                }
            }
        });

        let (_dc, _dr) = con.create_data_channel(DataChannelConfig {
            label: Some("data".into()),
        }).await?;

        let offer = con.create_offer(OfferConfig::default()).await?;
        con.set_local_description(offer).await?;

        let _ = r.await;

        let lock = result2.lock().unwrap();
        if lock.is_empty() {
            Result::Ok(NatType::UdpFailureTurnRequired)
        } else {
            let mut it = lock.values();
            if it.next().unwrap().len() == 1 {
                Result::Ok(NatType::NormalNatStunOkay)
            } else {
                Result::Ok(NatType::SymmetricNatTurnRequired)
            }
        }
    }.await {
        Ok(r) => r,
        Err(err) => {
            eprintln!("{err:?}");
            NatType::UdpFailureTurnRequired
        }
    }
}
