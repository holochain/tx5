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
    let subscriber = tracing_subscriber::FmtSubscriber::builder()
        .with_env_filter(
            tracing_subscriber::filter::EnvFilter::from_default_env(),
        )
        .with_file(true)
        .with_line_number(true)
        .finish();
    let _ = tracing::subscriber::set_global_default(subscriber);

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

    let (c1, mut evt1) = spawn_peer(config.clone()).await;
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
        let (data_chan, data_recv) = c1
            .create_data_channel(DataChannelConfig {
                label: Some("data".into()),
            })
            .await
            .unwrap();

        tokio::task::spawn(spawn_chan(
            data_chan,
            data_recv,
            start,
            chan_ready1,
            rcv_done1,
        ));

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
                    PeerConnectionState::Connecting,
                )) => (),
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

    let (c2, mut evt2) = spawn_peer(config.clone()).await;
    tokio::task::spawn(async move {
        while let Some(evt) = evt2.recv().await {
            t2t_snd.send(Cmd::PeerEvt(evt)).unwrap();
        }
    });

    while let Some(cmd) = t_rcv.recv().await {
        match cmd {
            Cmd::PeerEvt(PeerConnectionEvent::State(
                PeerConnectionState::Connecting,
            )) => (),
            Cmd::PeerEvt(PeerConnectionEvent::State(
                PeerConnectionState::Connected,
            )) => (),
            Cmd::PeerEvt(PeerConnectionEvent::ICECandidate(mut ice)) => {
                if is_ice_relay(&mut ice) {
                    t2o_snd.send(Cmd::Ice(ice)).unwrap();
                }
            }
            Cmd::PeerEvt(PeerConnectionEvent::DataChannel(
                data_chan,
                data_recv,
            )) => {
                tokio::task::spawn(spawn_chan(
                    data_chan,
                    data_recv,
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
    let (con, rcv) = PeerConnection::new(config).await.unwrap();
    (con, rcv)
}

async fn spawn_chan(
    data_chan: DataChannel,
    mut data_recv: tokio::sync::mpsc::UnboundedReceiver<DataChannelEvent>,
    start: std::time::Instant,
    chan_ready: Arc<tokio::sync::Barrier>,
    rcv_done: Arc<tokio::sync::Barrier>,
) {
    loop {
        match data_recv.recv().await {
            Some(DataChannelEvent::Open) => break,
            Some(DataChannelEvent::BufferedAmountLow) => (),
            oth => panic!("{oth:?}"),
        }
    }

    println!("chan ready");

    chan_ready.wait().await;

    print_chan_ready_time(start);

    for _ in 0..MSG_CNT {
        let buf = GoBuf::from_slice(ONE_KB).unwrap();
        data_chan.send(buf).await.unwrap();
    }

    let mut cnt = 0;

    loop {
        match data_recv.recv().await {
            Some(DataChannelEvent::Open) => (),
            Some(DataChannelEvent::BufferedAmountLow) => (),
            Some(DataChannelEvent::Message(mut buf)) => {
                assert_eq!(1024, buf.len().unwrap());
                std::io::Write::write_all(&mut std::io::stdout(), b".")
                    .unwrap();
                std::io::Write::flush(&mut std::io::stdout()).unwrap();
                cnt += 1;
                if cnt == MSG_CNT {
                    break;
                }
            }
            oth => panic!("{oth:?}"),
        }
    }

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
