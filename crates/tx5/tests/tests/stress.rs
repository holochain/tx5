use crate::tests::enable_tracing;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use tx5::*;

#[tokio::test(flavor = "multi_thread")]
#[cfg_attr(not(target_os = "linux"), ignore = "flaky on non-linux")]
async fn stress_small_msg() {
    stress_msg_size(20000, 42).await;
}

#[tokio::test(flavor = "multi_thread")]
#[cfg_attr(not(target_os = "linux"), ignore = "flaky on non-linux")]
async fn stress_large_msg() {
    stress_msg_size(5000, 1024 * 30).await;
}

async fn stress_msg_size(msg_count: usize, size: usize) {
    enable_tracing();

    let msg = vec![0xdb; size];

    let test = Test::new().await;

    let ep1 = test.ep().await;
    let ep2 = test.ep().await;

    for i in 0..msg_count {
        if i % (msg_count / 10) == 0 {
            println!("send msg {i} of {msg_count}");
        }
        ep1.ep
            .send(ep2.peer_url.clone(), msg.clone())
            .await
            .unwrap();
    }

    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        let mut got = 0;
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;

            let count = ep2.messages.lock().unwrap().len();
            if count != got {
                got = count;
                println!("recv msg {got} of {msg_count}");
            }
            if count == msg_count {
                // test pass
                return;
            }
        }
    })
    .await
    .unwrap();
}

struct TestEp {
    drop: Arc<AtomicBool>,
    peer_url: PeerUrl,
    task: tokio::task::AbortHandle,
    messages: Arc<Mutex<Vec<String>>>,
    ep: Endpoint,
}

impl Drop for TestEp {
    fn drop(&mut self) {
        self.drop.store(true, Ordering::SeqCst);
        self.task.abort();
    }
}

impl TestEp {
    pub async fn new(
        peer_url: PeerUrl,
        ep: Endpoint,
        mut ep_recv: EndpointRecv,
    ) -> Self {
        let drop = Arc::new(AtomicBool::new(false));
        let drop2 = drop.clone();

        let messages = Arc::new(Mutex::new(Vec::new()));
        let messages2 = messages.clone();

        let task = tokio::task::spawn(async move {
            while let Some(evt) = ep_recv.recv().await {
                match evt {
                    EndpointEvent::Disconnected { .. } => {
                        if !drop2.load(Ordering::SeqCst) {
                            panic!("disconnected");
                        }
                    }
                    EndpointEvent::Message { message, .. } => {
                        messages2
                            .lock()
                            .unwrap()
                            .push(String::from_utf8_lossy(&message).into());
                    }
                    _ => (),
                }
            }
        })
        .abort_handle();

        Self {
            drop,
            peer_url,
            task,
            messages,
            ep,
        }
    }
}

struct Test {
    _server: sbd_server::SbdServer,
    sig_url: SigUrl,
}

impl Test {
    pub async fn new() -> Self {
        let config = Arc::new(sbd_server::Config {
            bind: vec!["127.0.0.1:0".into(), "[::1]:0".into()],
            disable_rate_limiting: true,
            ..Default::default()
        });

        let server = sbd_server::SbdServer::new(config).await.unwrap();
        let sig_url = format!("ws://{}", server.bind_addrs().get(0).unwrap());
        let sig_url = SigUrl::parse(sig_url).unwrap();

        Self {
            _server: server,
            sig_url,
        }
    }

    pub async fn ep(&self) -> TestEp {
        let config = Arc::new(Config {
            signal_allow_plain_text: true,
            ..Default::default()
        });
        let (ep, ep_recv) = Endpoint::new(config);
        let peer_url = ep.listen(self.sig_url.clone()).await.unwrap();

        TestEp::new(peer_url, ep, ep_recv).await
    }
}
