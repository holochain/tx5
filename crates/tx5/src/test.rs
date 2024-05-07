use crate::*;

struct TestEp {
    ep: Endpoint,
    task: tokio::task::JoinHandle<()>,
    recv: Option<tokio::sync::mpsc::UnboundedReceiver<(PeerUrl, Vec<u8>)>>,
    peer_url: Arc<Mutex<PeerUrl>>,
}

impl std::ops::Deref for TestEp {
    type Target = Endpoint;

    fn deref(&self) -> &Self::Target {
        &self.ep
    }
}

impl Drop for TestEp {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl TestEp {
    pub async fn new(ep: Endpoint, mut ep_recv: EndpointRecv) -> Self {
        let (send, recv) = tokio::sync::mpsc::unbounded_channel();

        let peer_url = Arc::new(Mutex::new(
            PeerUrl::parse(
                "ws://bad/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
            )
            .unwrap(),
        ));
        let (s, r) = tokio::sync::oneshot::channel();
        let mut s = Some(s);

        let peer_url2 = peer_url.clone();
        let task = tokio::task::spawn(async move {
            while let Some(evt) = ep_recv.recv().await {
                match evt {
                    EndpointEvent::ListeningAddressOpen { local_url } => {
                        *peer_url2.lock().unwrap() = local_url;
                        if let Some(s) = s.take() {
                            let _ = s.send(());
                        }
                    }
                    EndpointEvent::Disconnected { peer_url } => {
                        if send
                            .send((peer_url, b"<<<test-disconnect>>>".to_vec()))
                            .is_err()
                        {
                            break;
                        }
                    }
                    EndpointEvent::Message { peer_url, message } => {
                        if send.send((peer_url, message)).is_err() {
                            break;
                        }
                    }
                    _ => (),
                }
            }
        });

        r.await.unwrap();

        Self {
            ep,
            task,
            recv: Some(recv),
            peer_url,
        }
    }

    fn peer_url(&self) -> PeerUrl {
        self.peer_url.lock().unwrap().clone()
    }

    async fn recv(&mut self) -> Option<(PeerUrl, Vec<u8>)> {
        match self.recv.as_mut() {
            Some(recv) => recv.recv().await,
            None => None,
        }
    }

    fn take_recv(
        &mut self,
    ) -> Option<tokio::sync::mpsc::UnboundedReceiver<(PeerUrl, Vec<u8>)>> {
        self.recv.take()
    }
}

struct Test {
    sig_srv_hnd: Option<sbd_server::SbdServer>,
    sig_port: Option<u16>,
    sig_url: Option<SigUrl>,
}

impl Test {
    pub async fn new() -> Self {
        let subscriber = tracing_subscriber::FmtSubscriber::builder()
            .with_env_filter(
                tracing_subscriber::filter::EnvFilter::from_default_env(),
            )
            .with_file(true)
            .with_line_number(true)
            .finish();

        let _ = tracing::subscriber::set_global_default(subscriber);

        let mut this = Test {
            sig_srv_hnd: None,
            sig_port: None,
            sig_url: None,
        };

        this.restart_sig().await;

        this
    }

    pub async fn ep(&self, config: Arc<Config>) -> TestEp {
        let sig_url = self.sig_url.clone().unwrap();

        let (ep, ep_recv) = Endpoint::new(config);
        ep.listen(sig_url);

        TestEp::new(ep, ep_recv).await
    }

    pub fn drop_sig(&mut self) {
        drop(self.sig_srv_hnd.take());
    }

    pub async fn restart_sig(&mut self) {
        self.drop_sig();

        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        let port = self.sig_port.unwrap_or(0);

        let bind = vec![format!("127.0.0.1:{port}"), format!("[::1]:{port}")];

        let config = Arc::new(sbd_server::Config {
            bind,
            disable_rate_limiting: true,
            ..Default::default()
        });

        let server = sbd_server::SbdServer::new(config).await.unwrap();

        let sig_port = server.bind_addrs().get(0).unwrap().port();
        self.sig_port = Some(sig_port);

        let sig_url = format!("ws://{}", server.bind_addrs().get(0).unwrap());
        let sig_url = SigUrl::parse(sig_url).unwrap();

        if let Some(old_sig_url) = &self.sig_url {
            if old_sig_url != &sig_url {
                panic!("mismatching new sig url");
            }
        }

        eprintln!("sig_url: {sig_url}");
        self.sig_url = Some(sig_url);

        self.sig_srv_hnd = Some(server);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn ep_sanity() {
    let config = Arc::new(Config {
        signal_allow_plain_text: true,
        ..Default::default()
    });
    let test = Test::new().await;

    let ep1 = test.ep(config.clone()).await;
    let mut ep2 = test.ep(config).await;

    ep1.send(ep2.peer_url(), b"hello".to_vec()).await.unwrap();

    let (from, msg) = ep2.recv().await.unwrap();
    assert_eq!(ep1.peer_url(), from);
    assert_eq!(&b"hello"[..], &msg);

    //let stats = ep1.get_stats().await;

    //println!("STATS: {}", serde_json::to_string_pretty(&stats).unwrap());
}

#[tokio::test(flavor = "multi_thread")]
async fn ep_sig_down() {
    eprintln!("-- STARTUP --");

    const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

    let config = Config {
        signal_allow_plain_text: true,
        timeout: TIMEOUT * 2,
        backoff_start: std::time::Duration::from_millis(2000),
        backoff_max: std::time::Duration::from_millis(2000),
        ..Default::default()
    };
    let config = Arc::new(config);
    let mut test = Test::new().await;

    let ep1 = test.ep(config.clone()).await;
    let mut ep2 = test.ep(config.clone()).await;

    eprintln!("-- Establish Connection --");

    ep1.send(ep2.peer_url(), b"hello".to_vec()).await.unwrap();

    let (from, msg) = ep2.recv().await.unwrap();
    assert_eq!(ep1.peer_url(), from);
    assert_eq!(&b"hello"[..], &msg);

    eprintln!("-- Drop Sig --");

    test.drop_sig();

    tokio::time::sleep(TIMEOUT).await;

    eprintln!("-- Send Should Fail --");

    ep1.send(ep2.peer_url(), b"hello".to_vec())
        .await
        .unwrap_err();

    eprintln!("-- Restart Sig --");

    test.restart_sig().await;

    tokio::time::sleep(TIMEOUT).await;

    eprintln!("-- Send Should Succeed --");

    ep1.send(ep2.peer_url(), b"hello".to_vec()).await.unwrap();

    loop {
        let (from, msg) = ep2.recv().await.unwrap();
        if &msg[..3] == b"<<<" {
            continue;
        }
        assert_eq!(ep1.peer_url(), from);
        assert_eq!(&b"hello"[..], &msg);
        break;
    }

    eprintln!("-- Done --");
}

#[tokio::test(flavor = "multi_thread")]
async fn ep_drop() {
    let config = Arc::new(Config {
        signal_allow_plain_text: true,
        ..Default::default()
    });
    let test = Test::new().await;

    let ep1 = test.ep(config.clone()).await;
    let mut ep2 = test.ep(config.clone()).await;

    ep1.send(ep2.peer_url(), b"hello".to_vec()).await.unwrap();

    let (from, msg) = ep2.recv().await.unwrap();
    assert_eq!(ep1.peer_url(), from);
    assert_eq!(&b"hello"[..], &msg);

    drop(ep2);

    let mut ep3 = test.ep(config).await;

    ep1.send(ep3.peer_url(), b"world".to_vec()).await.unwrap();

    let (from, msg) = ep3.recv().await.unwrap();
    assert_eq!(ep1.peer_url(), from);
    assert_eq!(&b"world"[..], &msg);
}

/// Test negotiation (polite / impolite node logic) by setting up a lot
/// of nodes and having them all try to make connections to each other
/// at the same time and see if we get all the messages.
#[tokio::test(flavor = "multi_thread")]
async fn ep_negotiation() {
    const NODE_COUNT: usize = 9;

    let mut url_list = Vec::new();
    let mut ep_list = Vec::new();

    let config = Arc::new(Config {
        signal_allow_plain_text: true,
        ..Default::default()
    });
    let test = Test::new().await;

    let mut fut_list = Vec::new();
    for _ in 0..NODE_COUNT {
        fut_list.push(test.ep(config.clone()));
    }

    for ep in futures::future::join_all(fut_list).await {
        url_list.push(ep.peer_url());
        ep_list.push(ep);
    }

    let first_url = url_list.get(0).unwrap().clone();

    // first, make sure all the connections are active
    // by connecting to the first node
    let mut fut_list = Vec::new();
    for (i, ep) in ep_list.iter_mut().enumerate() {
        if i != 0 {
            fut_list.push(ep.send(first_url.clone(), b"hello".to_vec()));
        }
    }

    for r in futures::future::join_all(fut_list).await {
        r.unwrap();
    }

    // now send messages between all the nodes
    let mut fut_list = Vec::new();
    for (i, ep) in ep_list.iter_mut().enumerate() {
        for (j, url) in url_list.iter().enumerate() {
            if i != j {
                fut_list.push(ep.send(url.clone(), b"world".to_vec()));
            }
        }
    }

    for r in futures::future::join_all(fut_list).await {
        r.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn ep_messages_contiguous() {
    let config = Arc::new(Config {
        signal_allow_plain_text: true,
        ..Default::default()
    });
    let test = Test::new().await;

    let mut ep = test.ep(config.clone()).await;
    let dest_url = ep.peer_url();
    let mut dest_recv = ep.take_recv().unwrap();

    const NODE_COUNT: usize = 3; // 3 nodes
    const SEND_COUNT: usize = 10; // sending 10 messages
    const CHUNK_COUNT: usize = 10; // broken into 10 chunks

    let mut all_tasks = Vec::new();

    let start = Arc::new(tokio::sync::Barrier::new(NODE_COUNT));
    let stop = Arc::new(tokio::sync::Barrier::new(NODE_COUNT + 1));

    for node_id in 0..NODE_COUNT {
        let dest_url = dest_url.clone();
        let start = start.clone();
        let stop = stop.clone();
        let ep = test.ep(config.clone()).await;
        all_tasks.push(tokio::task::spawn(async move {
            let mut messages = Vec::new();

            for msg_id in 0..SEND_COUNT {
                let mut chunks = vec![b'-'; ((16 * 1024) - 4) * CHUNK_COUNT];

                for chunk_id in 0..CHUNK_COUNT {
                    let data = format!("{node_id}:{msg_id}:{chunk_id}");
                    let data = data.as_bytes();
                    let s = ((16 * 1024) - 4) * chunk_id;
                    chunks[s..s + data.len()].copy_from_slice(data);
                }

                messages.push(chunks);
            }

            println!("{node_id} start wait");
            start.wait().await;

            let mut msg_id = 0;

            println!("{node_id} writing");
            for message in messages {
                msg_id += 1;

                println!("{node_id} writing message {msg_id}");
                ep.send(dest_url.clone(), message).await.unwrap();
            }

            println!("{node_id} stop wait");
            stop.wait().await;
            println!("{node_id} done");
        }));
    }

    let mut sort: HashMap<String, Vec<(usize, usize, usize)>> = HashMap::new();

    let mut count = 0;

    loop {
        let (peer_url, message) = dest_recv.recv().await.unwrap();

        assert_eq!(((16 * 1024) - 4) * CHUNK_COUNT, message.len());

        for chunk_id in 0..CHUNK_COUNT {
            let s = ((16 * 1024) - 4) * chunk_id;
            let s = String::from_utf8_lossy(&message[s..s + 32]);
            let mut s = s.split("-");
            let s = s.next().unwrap();
            let mut parts = s.split(':');
            let node = parts.next().unwrap().parse().unwrap();
            let msg = parts.next().unwrap().parse().unwrap();
            let chunk = parts.next().unwrap().parse().unwrap();
            sort.entry(peer_url.to_string())
                .or_default()
                .push((node, msg, chunk));
        }

        count += 1;

        if count >= NODE_COUNT * SEND_COUNT {
            break;
        }
    }

    println!("{sort:?}");

    // make sure the there is no cross-messaging
    for (_, list) in sort.iter() {
        let (check_node_id, _, _) = list.get(0).unwrap();

        for (node_id, _, _) in list.iter() {
            assert_eq!(check_node_id, node_id);
        }
    }

    // make sure msg/chunk strictly ascend
    for (_, list) in sort.iter() {
        let mut expect_msg = 0;
        let mut expect_chunk = 0;

        for (_, msg, chunk) in list.iter() {
            //println!("msg: {expect_msg}=={msg}, chunk: {expect_chunk}=={chunk}");

            assert_eq!(expect_msg, *msg);
            assert_eq!(expect_chunk, *chunk);

            expect_chunk += 1;
            if expect_chunk >= CHUNK_COUNT {
                expect_msg += 1;
                expect_chunk = 0;
            }
        }
    }

    stop.wait().await;

    for task in all_tasks {
        task.await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn ep_preflight_happy() {
    let did_send = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let did_valid = Arc::new(std::sync::atomic::AtomicBool::new(false));

    let preflight = vec![0xdb; 17 * 1024];

    let pf_send: PreflightSendCb = {
        let did_send = did_send.clone();
        let preflight = preflight.clone();
        Arc::new(move |_| {
            did_send.store(true, std::sync::atomic::Ordering::SeqCst);
            let preflight = preflight.clone();
            Box::pin(async move { Ok(preflight) })
        })
    };

    let pf_check: PreflightCheckCb = {
        let did_valid = did_valid.clone();
        Arc::new(move |_, bytes| {
            did_valid.store(true, std::sync::atomic::Ordering::SeqCst);
            assert_eq!(preflight, bytes);
            Box::pin(async move { Ok(()) })
        })
    };

    let config = Arc::new(Config {
        signal_allow_plain_text: true,
        preflight: Some((pf_send, pf_check)),
        ..Default::default()
    });

    let test = Test::new().await;

    let ep1 = test.ep(config.clone()).await;
    let mut ep2 = test.ep(config).await;
    let mut ep2_recv = ep2.take_recv().unwrap();

    ep1.send(ep2.peer_url(), b"hello".to_vec()).await.unwrap();

    let (_, message) = ep2_recv.recv().await.unwrap();
    assert_eq!(&b"hello"[..], &message);

    assert_eq!(true, did_send.load(std::sync::atomic::Ordering::SeqCst));
    assert_eq!(true, did_valid.load(std::sync::atomic::Ordering::SeqCst));
}

#[tokio::test(flavor = "multi_thread")]
async fn ep_close_connection() {
    let config = Arc::new(Config {
        signal_allow_plain_text: true,
        timeout: std::time::Duration::from_secs(2),
        ..Default::default()
    });
    let test = Test::new().await;

    let ep1 = test.ep(config.clone()).await;
    let mut ep2 = test.ep(config).await;
    let mut ep2_recv = ep2.take_recv().unwrap();

    ep1.send(ep2.peer_url(), b"hello".to_vec()).await.unwrap();

    let (_, message) = ep2_recv.recv().await.unwrap();
    assert_eq!(&b"hello"[..], &message);

    ep1.close(&ep2.peer_url());

    let (url, message) = ep2_recv.recv().await.unwrap();
    assert_eq!(ep1.peer_url(), url);
    assert_eq!(&b"<<<test-disconnect>>>"[..], &message);
}

#[tokio::test(flavor = "multi_thread")]
async fn ep_broadcast_happy() {
    let config = Arc::new(Config {
        signal_allow_plain_text: true,
        ..Default::default()
    });
    let test = Test::new().await;

    let ep1 = test.ep(config.clone()).await;
    let mut ep2 = test.ep(config.clone()).await;
    let mut ep2_recv = ep2.take_recv().unwrap();
    let mut ep3 = test.ep(config).await;
    let mut ep3_recv = ep3.take_recv().unwrap();

    ep1.send(ep2.peer_url(), b"hello".to_vec()).await.unwrap();

    ep1.send(ep3.peer_url(), b"hello".to_vec()).await.unwrap();

    let (_, message) = ep2_recv.recv().await.unwrap();
    assert_eq!(&b"hello"[..], &message);

    let (_, message) = ep3_recv.recv().await.unwrap();
    assert_eq!(&b"hello"[..], &message);

    ep1.broadcast(b"world").await;

    let (_, message) = ep2_recv.recv().await.unwrap();
    assert_eq!(&b"world"[..], &message);

    let (_, message) = ep3_recv.recv().await.unwrap();
    assert_eq!(&b"world"[..], &message);
}
