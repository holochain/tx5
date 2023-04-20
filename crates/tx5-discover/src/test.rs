use super::*;

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
async fn shotgun_can_receive_announce() {
    const THIS_TEST_PORT: u16 = 13132;

    init_tracing();

    let (recv_s, mut recv_r) = tokio::sync::mpsc::unbounded_channel();

    let sg = Shotgun::new(
        Arc::new(move |res| {
            let _ = recv_s.send(res);
        }),
        Arc::new(|_| ()),
        THIS_TEST_PORT,
        MULTICAST_V4,
        MULTICAST_V6,
    )
    .await
    .unwrap();

    sg.multicast(b"hello".to_vec()).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    sg.multicast(b"hello".to_vec()).await.unwrap();

    let got_v4 = std::sync::atomic::AtomicBool::new(false);
    let got_v4 = &got_v4;
    let got_v6 = std::sync::atomic::AtomicBool::new(false);
    let got_v6 = &got_v6;

    trait EZ {
        fn set(&self, v: bool);
        fn get(&self) -> bool;
    }

    impl EZ for std::sync::atomic::AtomicBool {
        fn set(&self, v: bool) {
            self.store(v, std::sync::atomic::Ordering::SeqCst);
        }
        fn get(&self) -> bool {
            self.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    tokio::select! {
        _ = tokio::time::sleep(std::time::Duration::from_secs(10)) => (),
        _ = async move {
            while let Some(res) = recv_r.recv().await {
                let (_, data, addr) = res.unwrap();
                assert_eq!(b"hello", data.as_slice());
                println!("{addr:?}");
                if addr.is_ipv4() {
                    got_v4.set(true);
                    if got_v6.get() == true {
                        break;
                    }
                }
                if addr.is_ipv6() {
                    got_v6.set(true);
                    if got_v4.get() == true {
                        break;
                    }
                }
            }

        } => (),
    }

    if !got_v4.get() && !got_v6.get() {
        panic!("no multicast received");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn discover_sanity() {
    const THIS_TEST_PORT: u16 = 13133;

    init_tracing();

    let config = Arc::new(
        Tx5DiscoverConfig::default()
            .with_port(THIS_TEST_PORT)
            .with_recv_cb(|evt| {
                println!("{evt:?}");
            }),
    );

    let d1 = Tx5Discover::new(config.clone(), vec![b"hello".to_vec()]).await.unwrap();
    let d2 = Tx5Discover::new(config, vec![b"world".to_vec()]).await.unwrap();

    tokio::time::sleep(std::time::Duration::from_secs(5)).await;

    d1.update_discovery_data(vec![b"foo".to_vec()]).unwrap();
    d2.update_discovery_data(vec![b"bar".to_vec()]).unwrap();

    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
}
