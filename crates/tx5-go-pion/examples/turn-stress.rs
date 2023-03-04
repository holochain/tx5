use tx5_go_pion::*;

const ONE_KB: [u8; 1024] = [0xdb; 1024];

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let mut args = std::env::args();

    args.next().unwrap();

    let host = args.next().expect("host");
    let port = args.next().expect("port");
    let user = args.next().expect("user");
    let cred = args.next().expect("cred");

    let ice = IceServer {
        urls: vec![format!("turn:{host}:{port}?transport=udp")],
        username: Some(user),
        credential: Some(cred),
    };

    let config = PeerConnectionConfig {
        ice_servers: vec![ice],
    };

    println!("running with: {config:?}");

    #[derive(Debug)]
    enum Cmd {
        PeerEvt(PeerConnectionEvent),
        Offer(GoBuf),
        Answer(GoBuf),
        Ice(GoBuf),
    }

    let (o2t_snd, mut t_rcv) = tokio::sync::mpsc::unbounded_channel();
    let (t2o_snd, mut o_rcv) = tokio::sync::mpsc::unbounded_channel();

    let o2o_snd = t2o_snd.clone();
    let t2t_snd = o2t_snd.clone();

    let (mut c1, mut evt1) = spawn_peer(config.clone()).await;
    tokio::task::spawn(async move {
        while let Some(evt) = evt1.recv().await {
            o2o_snd.send(Cmd::PeerEvt(evt)).unwrap();
        }
    });

    tokio::task::spawn(async move {
        let seed = c1.create_data_channel(DataChannelConfig {
            label: Some("data".into()),
        }).await.unwrap();

        tokio::task::spawn(spawn_chan(seed));

        let mut offer = c1.create_offer(OfferConfig::default()).await.unwrap();

        println!("created offer: {:?}", String::from_utf8_lossy(&offer.to_vec().unwrap()));

        c1.set_local_description(&mut offer).await.unwrap();

        o2t_snd.send(Cmd::Offer(offer)).unwrap();

        let mut ice_buf = Some(Vec::new());

        while let Some(cmd) = o_rcv.recv().await {
            match cmd {
                Cmd::PeerEvt(PeerConnectionEvent::ICECandidate(mut ice)) => {
                    if is_ice_relay(&mut ice) {
                        o2t_snd.send(Cmd::Ice(ice)).unwrap();
                    }
                }
                Cmd::Answer(answer) => {
                    c1.set_remote_description(answer).await.unwrap();
                    if let Some(ice_buf) = ice_buf.take() {
                        for ice in ice_buf {
                            c1.add_ice_candidate(ice).await.unwrap();
                        }
                    }
                }
                Cmd::Ice(ice) => {
                    if let Some(ice_buf) = ice_buf.as_mut() {
                        ice_buf.push(ice);
                    } else {
                        c1.add_ice_candidate(ice).await.unwrap();
                    }
                }
                _ => panic!("unexpected"),
            }
        }
    });

    let mut ice_buf = Some(Vec::new());

    let (mut c2, mut evt2) = spawn_peer(config.clone()).await;
    tokio::task::spawn(async move {
        while let Some(evt) = evt2.recv().await {
            t2t_snd.send(Cmd::PeerEvt(evt)).unwrap();
        }
    });

    while let Some(cmd) = t_rcv.recv().await {
        match cmd {
            Cmd::PeerEvt(PeerConnectionEvent::ICECandidate(mut ice)) => {
                if is_ice_relay(&mut ice) {
                    t2o_snd.send(Cmd::Ice(ice)).unwrap();
                }
            }
            Cmd::PeerEvt(PeerConnectionEvent::DataChannel(seed)) => {
                tokio::task::spawn(spawn_chan(seed));
            }
            Cmd::Offer(offer) => {
                c2.set_remote_description(offer).await.unwrap();
                let mut answer = c2.create_answer(AnswerConfig::default()).await.unwrap();
                println!("created answer: {:?}", String::from_utf8_lossy(&answer.to_vec().unwrap()));
                c2.set_local_description(&mut answer).await.unwrap();
                t2o_snd.send(Cmd::Answer(answer)).unwrap();
                if let Some(ice_buf) = ice_buf.take() {
                    for ice in ice_buf {
                        c2.add_ice_candidate(ice).await.unwrap();
                    }
                }
            }
            Cmd::Ice(ice) => {
                if let Some(ice_buf) = ice_buf.as_mut() {
                    ice_buf.push(ice);
                } else {
                    c2.add_ice_candidate(ice).await.unwrap();
                }
            }
            _ => panic!("unexpected"),
        }
    }
}

async fn spawn_peer(config: PeerConnectionConfig) -> (PeerConnection, tokio::sync::mpsc::UnboundedReceiver<PeerConnectionEvent>) {
    let (snd, rcv) = tokio::sync::mpsc::unbounded_channel();
    let con = PeerConnection::new(config, move |evt| {
        let _ = snd.send(evt);
    }).await.unwrap();
    (con, rcv)
}

async fn spawn_chan(seed: DataChannelSeed) {
    let (s, r) = tokio::sync::oneshot::channel();
    let s = std::sync::Mutex::new(Some(s));
    let mut chan = seed.handle(move |evt| {
        match evt {
            DataChannelEvent::Close => (),
            DataChannelEvent::Open => {
                if let Some(s) = s.lock().unwrap().take() {
                    let _ = s.send(());
                }
            }
            DataChannelEvent::Message(mut buf) => {
                assert_eq!(1024, buf.len().unwrap());
                std::io::Write::write_all(&mut std::io::stdout(), b".").unwrap();
                std::io::Write::flush(&mut std::io::stdout()).unwrap();
            }
        }
    });

    //if chan.ready_state().unwrap() < 2 {
        r.await.unwrap();
    //}

    println!("chan ready");

    for _ in 0..1024 {
        let buf = GoBuf::from_slice(ONE_KB).unwrap();
        chan.send(buf).await.unwrap();
    }

    let (_s, r) = tokio::sync::oneshot::channel::<()>();
    let _ = r.await;
}

fn is_ice_relay(ice: &mut GoBuf) -> bool {
    let data = ice.to_vec().unwrap();
    let s = String::from_utf8_lossy(&data);
    if s.contains(" relay ") {
        println!("ICE: {s}");
        true
    } else {
        false
    }
}
