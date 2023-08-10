//! Usage: turn-stress hostname port user credential
//!  e.g.: turn-stress my-domain.com 11223 my-user my-password
//!
//! Sets up two webrtc endpoints using the provided turn server,
//! Filters ice on " relay ", so we'll be sure to go through the turn server.
//! Sends 1024 1024-byte messages (so 1MiB total) from each side (so 2MiB total)
//! Awaits the 1024 messages on each side.
//! Prints out timing info.

use std::sync::Arc;
use tx5_go_pion::*;

const ONE_KB: [u8; 1024] = [0xdb; 1024];
const MSG_CNT: usize = 1024;

fn print_chan_ready_time(start: std::time::Instant) {
    static CR: std::sync::Once = std::sync::Once::new();
    CR.call_once(move || {
        let elapsed = start.elapsed().as_secs_f64();
        println!("\nchan ready in {elapsed} seconds");
    });
}

fn print_rcv_done_time(start: std::time::Instant) {
    static RD: std::sync::Once = std::sync::Once::new();
    RD.call_once(move || {
        let elapsed = start.elapsed().as_secs_f64();
        println!("\nreceive done in {elapsed} seconds");
    });
}

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

    let start = std::time::Instant::now();

    let (mut c1, mut evt1) = spawn_peer(config.clone()).await;
    tokio::task::spawn(async move {
        while let Some(evt) = evt1.recv().await {
            o2o_snd.send(Cmd::PeerEvt(evt)).unwrap();
        }
    });

    let chan_ready = Arc::new(tokio::sync::Barrier::new(2));
    let chan_ready1 = chan_ready.clone();

    let rcv_done = Arc::new(tokio::sync::Barrier::new(2));
    let rcv_done1 = rcv_done.clone();

    tokio::task::spawn(async move {
        let seed = c1
            .create_data_channel(DataChannelConfig {
                label: Some("data".into()),
            })
            .await
            .unwrap();

        tokio::task::spawn(spawn_chan(seed, start, chan_ready1, rcv_done1));

        let mut offer = c1.create_offer(OfferConfig::default()).await.unwrap();

        println!(
            "created offer: {:?}",
            String::from_utf8_lossy(&offer.to_vec().unwrap())
        );

        c1.set_local_description(&mut offer).await.unwrap();

        o2t_snd.send(Cmd::Offer(offer)).unwrap();

        let mut ice_buf = Some(Vec::new());

        while let Some(cmd) = o_rcv.recv().await {
            match cmd {
                Cmd::PeerEvt(PeerConnectionEvent::State(
                    PeerConnectionState::Connected,
                )) => (),
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
                oth => panic!("unexpected: {oth:?}"),
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
            Cmd::PeerEvt(PeerConnectionEvent::State(
                PeerConnectionState::Connected,
            )) => (),
            Cmd::PeerEvt(PeerConnectionEvent::ICECandidate(mut ice)) => {
                if is_ice_relay(&mut ice) {
                    t2o_snd.send(Cmd::Ice(ice)).unwrap();
                }
            }
            Cmd::PeerEvt(PeerConnectionEvent::DataChannel(seed)) => {
                tokio::task::spawn(spawn_chan(
                    seed,
                    start,
                    chan_ready.clone(),
                    rcv_done.clone(),
                ));
            }
            Cmd::Offer(offer) => {
                c2.set_remote_description(offer).await.unwrap();
                let mut answer =
                    c2.create_answer(AnswerConfig::default()).await.unwrap();
                println!(
                    "created answer: {:?}",
                    String::from_utf8_lossy(&answer.to_vec().unwrap())
                );
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
            oth => panic!("unexpected: {oth:?}"),
        }
    }
}

async fn spawn_peer(
    config: PeerConnectionConfig,
) -> (
    PeerConnection,
    tokio::sync::mpsc::UnboundedReceiver<PeerConnectionEvent>,
) {
    let (snd, rcv) = tokio::sync::mpsc::unbounded_channel();
    let con = PeerConnection::new(config, move |evt| {
        let _ = snd.send(evt);
    })
    .await
    .unwrap();
    (con, rcv)
}

async fn spawn_chan(
    seed: DataChannelSeed,
    start: std::time::Instant,
    chan_ready: Arc<tokio::sync::Barrier>,
    rcv_done: Arc<tokio::sync::Barrier>,
) {
    let (s_o, r_o) = tokio::sync::oneshot::channel();
    let s_o = std::sync::Mutex::new(Some(s_o));
    let (s_d, r_d) = tokio::sync::oneshot::channel();
    let s_d = std::sync::Mutex::new(Some(s_d));
    let c = std::sync::atomic::AtomicUsize::new(1);
    let mut chan = seed.handle(move |evt| match evt {
        DataChannelEvent::Close | DataChannelEvent::BufferedAmountLow => (),
        DataChannelEvent::Open => {
            if let Some(s_o) = s_o.lock().unwrap().take() {
                let _ = s_o.send(());
            }
        }
        DataChannelEvent::Message(mut buf) => {
            assert_eq!(1024, buf.len().unwrap());
            std::io::Write::write_all(&mut std::io::stdout(), b".").unwrap();
            std::io::Write::flush(&mut std::io::stdout()).unwrap();
            let cnt = c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if cnt == MSG_CNT {
                if let Some(s_d) = s_d.lock().unwrap().take() {
                    let _ = s_d.send(());
                }
            }
        }
    });

    //if chan.ready_state().unwrap() < 2 {
    r_o.await.unwrap();
    //}

    println!("chan ready");

    chan_ready.wait().await;

    print_chan_ready_time(start);

    for _ in 0..MSG_CNT {
        let buf = GoBuf::from_slice(ONE_KB).unwrap();
        chan.send(buf).await.unwrap();
    }

    // we've received all our messages
    r_d.await.unwrap();

    rcv_done.wait().await;

    println!("\nreceive complete");

    print_rcv_done_time(start);

    std::process::exit(0);
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
