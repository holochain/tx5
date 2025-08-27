//! Mem backend timings. The hope is to have near ideal zero overhead.
//! At time of writing, you can see we're pretty close to exactly
//! proportional to node count for a given machine's resources:
//!
//! - 1 nodes, 33941 ops, 0.000028 sec/op, in 1.004478s
//! - 10 nodes, 174041 ops, 0.000056 sec/op, in 1.016977s
//! - 100 nodes, 175111 ops, 0.000580 sec/op, in 1.018943s
//! - 1000 nodes, 160266 ops, 0.006361 sec/op, in 1.019859s

use std::sync::Arc;
use tx5::{backend::*, *};

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let fake_sig = SigUrl::parse("wss://fake.fake").unwrap();

    let config =
        Arc::new(Config::new().with_backend_module(BackendModule::Mem));

    let (listen_ep, mut listen_recv) = Endpoint::new(config.clone());
    let listen_ep = Arc::new(listen_ep);
    let listen_url = listen_ep.listen(fake_sig.clone()).await.unwrap();

    tokio::task::spawn(async move {
        while let Some(evt) = listen_recv.recv().await {
            match evt {
                EndpointEvent::ListeningAddressOpen { .. } => (),
                EndpointEvent::Connected { .. } => (),
                EndpointEvent::Message { peer_url, .. } => {
                    let listen_ep = listen_ep.clone();
                    // Don't hold up our recv loop on the response
                    tokio::task::spawn(async move {
                        listen_ep.send(peer_url, Vec::new()).await.unwrap();
                    });
                }
                _ => panic!("unexpected: {evt:?}"),
            }
        }
        panic!("listener task ended");
    });

    println!("listening at: {listen_url}");

    let (timing_send, mut timing_recv) = tokio::sync::mpsc::unbounded_channel();
    let mut timing_buf = Vec::new();
    let mut node_count = 0;

    loop {
        let start = std::time::Instant::now();

        node_count += 1;

        let config = config.clone();
        let timing_send = timing_send.clone();
        let listen_url = listen_url.clone();
        tokio::task::spawn(async move {
            let (cli_ep, mut cli_recv) = Endpoint::new(config);
            let _ = cli_ep.listen(listen_url.to_sig()).await.unwrap();

            loop {
                let start = std::time::Instant::now();
                cli_ep.send(listen_url.clone(), Vec::new()).await.unwrap();
                loop {
                    let evt = cli_recv.recv().await.unwrap();
                    match evt {
                        EndpointEvent::ListeningAddressOpen { .. } => (),
                        EndpointEvent::Connected { .. } => (),
                        EndpointEvent::Message { .. } => break,
                        _ => panic!("unexpected: {evt:?}"),
                    }
                }
                timing_send.send(start.elapsed().as_secs_f64()).unwrap();
            }
        });

        tokio::time::sleep(std::time::Duration::from_secs(1)).await;

        timing_recv.recv_many(&mut timing_buf, usize::MAX).await;

        let count = timing_buf.len();
        let sum: f64 = timing_buf.iter().sum();
        timing_buf.clear();

        println!(
            "{node_count} nodes, {count} ops, {:0.6} sec/op, in {:0.6}s",
            sum / count as f64,
            start.elapsed().as_secs_f64(),
        );
    }
}
