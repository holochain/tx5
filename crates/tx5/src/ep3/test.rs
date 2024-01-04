use super::*;

struct Test {
    _sig_srv_hnd: tx5_signal_srv::SrvHnd,
    sig_url: SigUrl,
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

        let mut srv_config = tx5_signal_srv::Config::default();
        srv_config.port = 0;

        let (_sig_srv_hnd, addr_list, _) =
            tx5_signal_srv::exec_tx5_signal_srv(srv_config).unwrap();

        let sig_port = addr_list.get(0).unwrap().port();

        let sig_url =
            SigUrl::new(format!("ws://localhost:{}", sig_port)).unwrap();

        tracing::info!(%sig_url);

        Test {
            _sig_srv_hnd,
            sig_url,
        }
    }

    pub async fn ep(
        &self,
        config: Arc<Config3>,
    ) -> (PeerUrl, Ep3, EventRecv<Ep3Event>) {
        let (ep, recv) = Ep3::new(config).await;
        let url = ep.listen(self.sig_url.clone()).await.unwrap();

        (url, ep, recv)
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn ep3_sanity() {
    let config = Arc::new(Config3::default());
    let test = Test::new().await;

    let (_cli_url1, ep1, _ep1_recv) = test.ep(config.clone()).await;
    let (cli_url2, _ep2, mut ep2_recv) = test.ep(config).await;

    ep1.send(cli_url2, vec![BackBuf::from_slice(b"hello").unwrap()])
        .await
        .unwrap();

    let res = ep2_recv.recv().await.unwrap();
    match res {
        Ep3Event::Connected { .. } => (),
        _ => panic!(),
    }

    let res = ep2_recv.recv().await.unwrap();
    match res {
        Ep3Event::Message { mut message, .. } => {
            assert_eq!(&b"hello"[..], &message.to_vec().unwrap());
        }
        _ => panic!(),
    }

    let stats = ep1.get_stats().await;

    println!("STATS: {}", serde_json::to_string_pretty(&stats).unwrap());
}

#[tokio::test(flavor = "multi_thread")]
async fn ep3_drop() {
    let config = Arc::new(Config3::default());
    let test = Test::new().await;

    let (_cli_url1, ep1, _ep1_recv) = test.ep(config.clone()).await;
    let (cli_url2, ep2, mut ep2_recv) = test.ep(config.clone()).await;

    ep1.send(cli_url2, vec![BackBuf::from_slice(b"hello").unwrap()])
        .await
        .unwrap();

    let res = ep2_recv.recv().await.unwrap();
    match res {
        Ep3Event::Connected { .. } => (),
        _ => panic!(),
    }

    let res = ep2_recv.recv().await.unwrap();
    match res {
        Ep3Event::Message { mut message, .. } => {
            assert_eq!(&b"hello"[..], &message.to_vec().unwrap());
        }
        _ => panic!(),
    }

    drop(ep2);
    drop(ep2_recv);

    let (cli_url3, _ep3, mut ep3_recv) = test.ep(config).await;

    ep1.send(cli_url3, vec![BackBuf::from_slice(b"world").unwrap()])
        .await
        .unwrap();

    let res = ep3_recv.recv().await.unwrap();
    match res {
        Ep3Event::Connected { .. } => (),
        _ => panic!(),
    }

    let res = ep3_recv.recv().await.unwrap();
    match res {
        Ep3Event::Message { mut message, .. } => {
            assert_eq!(&b"world"[..], &message.to_vec().unwrap());
        }
        _ => panic!(),
    }
}

/// Test negotiation (polite / impolite node logic) by setting up a lot
/// of nodes and having them all try to make connections to each other
/// at the same time and see if we get all the messages.
#[tokio::test(flavor = "multi_thread")]
async fn ep3_negotiation() {
    const NODE_COUNT: usize = 9;

    let mut url_list = Vec::new();
    let mut ep_list = Vec::new();
    let mut recv_list = Vec::new();

    let config = Arc::new(Config3::default());
    let test = Test::new().await;

    let mut fut_list = Vec::new();
    for _ in 0..NODE_COUNT {
        fut_list.push(test.ep(config.clone()));
    }

    for (url, ep, recv) in futures::future::join_all(fut_list).await {
        url_list.push(url);
        ep_list.push(ep);
        recv_list.push(recv);
    }

    let first_url = url_list.get(0).unwrap().clone();

    // first, make sure all the connections are active
    // by connecting to the first node
    let mut fut_list = Vec::new();
    for (i, ep) in ep_list.iter_mut().enumerate() {
        if i != 0 {
            fut_list.push(ep.send(
                first_url.clone(),
                vec![BackBuf::from_slice(b"hello").unwrap()],
            ));
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
                fut_list.push(ep.send(
                    url.clone(),
                    vec![BackBuf::from_slice(b"world").unwrap()],
                ));
            }
        }
    }

    for r in futures::future::join_all(fut_list).await {
        r.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn ep3_messages_contiguous() {
    let config = Arc::new(Config3::default());
    let test = Test::new().await;

    let (dest_url, _dest_ep, mut dest_recv) = test.ep(config.clone()).await;

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
        let (_url, ep, _recv) = test.ep(config.clone()).await;
        all_tasks.push(tokio::task::spawn(async move {
            let _recv = _recv;

            start.wait().await;

            for msg_id in 0..SEND_COUNT {
                let mut chunks = Vec::new();

                for chunk_id in 0..CHUNK_COUNT {
                    chunks.push(
                        BackBuf::from_slice(
                            format!("{node_id}:{msg_id}:{chunk_id}").as_bytes(),
                        )
                        .unwrap(),
                    );
                }

                ep.send(dest_url.clone(), chunks).await.unwrap();
            }

            stop.wait().await;
        }));
    }

    let mut sort: HashMap<String, Vec<(usize, usize, usize)>> = HashMap::new();

    let mut count = 0;

    loop {
        let res = dest_recv.recv().await.unwrap();
        match res {
            Ep3Event::Connected { .. } => (),
            Ep3Event::Message {
                peer_url,
                mut message,
                ..
            } => {
                let message = message.to_vec().unwrap();
                let message = String::from_utf8_lossy(&message).to_string();
                let mut parts = message.split(':');
                let node = parts.next().unwrap().parse().unwrap();
                let msg = parts.next().unwrap().parse().unwrap();
                let chunk = parts.next().unwrap().parse().unwrap();

                sort.entry(peer_url.to_string())
                    .or_default()
                    .push((node, msg, chunk));

                count += 1;

                if count >= NODE_COUNT * SEND_COUNT * CHUNK_COUNT {
                    break;
                }
            }
            _ => panic!(),
        }
    }

    println!("{sort:?}");

    // make sure the there is no cross-messaging
    for (_, list) in sort.iter() {
        let (check_node_id, _, _) = list.get(0).unwrap();

        for (node_id, _, _) in list.iter() {
            //println!("{check_node_id}=={node_id}");
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
async fn ep3_preflight_happy() {
    let mut config = Config3::default();
    config.preflight_send_cb = Arc::new(|_| {
        Box::pin(async move {
            Ok(Some(vec![
                BackBuf::from_slice(b"0").unwrap(),
                BackBuf::from_slice(b"1").unwrap(),
                BackBuf::from_slice(b"2").unwrap(),
                BackBuf::from_slice(b"3").unwrap(),
            ]))
        })
    });
    config.preflight_check_cb = Arc::new(|_, bytes| {
        let res = if bytes.0.len() == 4 {
            let r = String::from_utf8_lossy(&bytes.to_vec()).to_string();
            assert_eq!("0123", r);
            PreflightCheckResponse::Valid
        } else {
            PreflightCheckResponse::NeedMoreData
        };
        Box::pin(async move { res })
    });
    let config = Arc::new(config);
    let test = Test::new().await;

    let (_cli_url1, ep1, _ep1_recv) = test.ep(config.clone()).await;
    let (cli_url2, _ep2, mut ep2_recv) = test.ep(config).await;

    ep1.send(cli_url2, vec![BackBuf::from_slice(b"hello").unwrap()])
        .await
        .unwrap();

    let res = ep2_recv.recv().await.unwrap();
    match res {
        Ep3Event::Connected { .. } => (),
        _ => panic!(),
    }

    let res = ep2_recv.recv().await.unwrap();
    match res {
        Ep3Event::Message { mut message, .. } => {
            assert_eq!(&b"hello"[..], &message.to_vec().unwrap());
        }
        _ => panic!(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn ep3_ban_after_connected_outgoing_side() {
    let config = Arc::new(Config3::default());
    let test = Test::new().await;

    let (_cli_url1, ep1, _ep1_recv) = test.ep(config.clone()).await;
    let (cli_url2, _ep2, mut ep2_recv) = test.ep(config).await;

    ep1.send(
        cli_url2.clone(),
        vec![BackBuf::from_slice(b"hello").unwrap()],
    )
    .await
    .unwrap();

    let res = ep2_recv.recv().await.unwrap();
    match res {
        Ep3Event::Connected { .. } => (),
        _ => panic!(),
    }

    let res = ep2_recv.recv().await.unwrap();
    match res {
        Ep3Event::Message { mut message, .. } => {
            assert_eq!(&b"hello"[..], &message.to_vec().unwrap());
        }
        _ => panic!(),
    }

    ep1.ban(cli_url2.id().unwrap(), std::time::Duration::from_secs(10));

    assert!(ep1
        .send(cli_url2, vec![BackBuf::from_slice(b"hello").unwrap()])
        .await
        .is_err());

    let stats = ep1.get_stats().await;

    println!("STATS: {}", serde_json::to_string_pretty(&stats).unwrap());
}

#[tokio::test(flavor = "multi_thread")]
async fn ep3_recon_after_ban() {
    let config = Arc::new(Config3::default());
    let test = Test::new().await;

    let (_cli_url1, ep1, _ep1_recv) = test.ep(config.clone()).await;
    let (cli_url2, _ep2, mut ep2_recv) = test.ep(config).await;

    ep1.send(
        cli_url2.clone(),
        vec![BackBuf::from_slice(b"hello").unwrap()],
    )
    .await
    .unwrap();

    let res = ep2_recv.recv().await.unwrap();
    match res {
        Ep3Event::Connected { .. } => (),
        _ => panic!(),
    }

    let res = ep2_recv.recv().await.unwrap();
    match res {
        Ep3Event::Message { mut message, .. } => {
            assert_eq!(&b"hello"[..], &message.to_vec().unwrap());
        }
        oth => panic!("{oth:?}"),
    }

    ep1.ban(cli_url2.id().unwrap(), std::time::Duration::from_millis(10));

    assert!(ep1
        .send(
            cli_url2.clone(),
            vec![BackBuf::from_slice(b"hello").unwrap()]
        )
        .await
        .is_err());

    let res = ep2_recv.recv().await.unwrap();
    match res {
        Ep3Event::Disconnected { .. } => (),
        oth => panic!("{oth:?}"),
    }

    tokio::time::sleep(std::time::Duration::from_millis(15)).await;

    ep1.send(
        cli_url2.clone(),
        vec![BackBuf::from_slice(b"world").unwrap()],
    )
    .await
    .unwrap();

    let res = ep2_recv.recv().await.unwrap();
    match res {
        Ep3Event::Connected { .. } => (),
        oth => panic!("{oth:?}"),
    }

    let res = ep2_recv.recv().await.unwrap();
    match res {
        Ep3Event::Message { mut message, .. } => {
            assert_eq!(&b"world"[..], &message.to_vec().unwrap());
        }
        _ => panic!(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn ep3_broadcast_happy() {
    let config = Arc::new(Config3::default());
    let test = Test::new().await;

    let (_cli_url1, ep1, _ep1_recv) = test.ep(config.clone()).await;
    let (cli_url2, _ep2, mut ep2_recv) = test.ep(config.clone()).await;
    let (cli_url3, _ep3, mut ep3_recv) = test.ep(config).await;

    ep1.send(
        cli_url2.clone(),
        vec![BackBuf::from_slice(b"hello").unwrap()],
    )
    .await
    .unwrap();

    ep1.send(
        cli_url3.clone(),
        vec![BackBuf::from_slice(b"hello").unwrap()],
    )
    .await
    .unwrap();

    let res = ep2_recv.recv().await.unwrap();
    match res {
        Ep3Event::Connected { .. } => (),
        _ => panic!(),
    }

    let res = ep2_recv.recv().await.unwrap();
    match res {
        Ep3Event::Message { mut message, .. } => {
            assert_eq!(&b"hello"[..], &message.to_vec().unwrap());
        }
        _ => panic!(),
    }

    let res = ep3_recv.recv().await.unwrap();
    match res {
        Ep3Event::Connected { .. } => (),
        _ => panic!(),
    }

    let res = ep3_recv.recv().await.unwrap();
    match res {
        Ep3Event::Message { mut message, .. } => {
            assert_eq!(&b"hello"[..], &message.to_vec().unwrap());
        }
        _ => panic!(),
    }

    ep1.broadcast(vec![BackBuf::from_slice(b"world").unwrap()])
        .await;

    let res = ep2_recv.recv().await.unwrap();
    match res {
        Ep3Event::Message { mut message, .. } => {
            assert_eq!(&b"world"[..], &message.to_vec().unwrap());
        }
        _ => panic!(),
    }

    let res = ep3_recv.recv().await.unwrap();
    match res {
        Ep3Event::Message { mut message, .. } => {
            assert_eq!(&b"world"[..], &message.to_vec().unwrap());
        }
        _ => panic!(),
    }
}
