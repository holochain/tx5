use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn sanity() {
    let (recv_s, mut recv_r) = tokio::sync::mpsc::unbounded_channel();

    let sg = Shotgun::new(
        Arc::new(move |res| {
            let _ = recv_s.send(res);
        }),
        PORT,
        MULTICAST_V4,
        MULTICAST_V6,
    )
    .await
    .unwrap();

    sg.multicast(b"hello".to_vec()).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    sg.multicast(b"hello".to_vec()).await.unwrap();

    tokio::select! {
        _ = tokio::time::sleep(std::time::Duration::from_secs(10)) => panic!("timeout"),
        _ = async move {
            let mut got_v4 = false;
            let mut got_v6 = false;

            while let Some(res) = recv_r.recv().await {
                let (data, addr) = res.unwrap();
                assert_eq!(b"hello", data.as_slice());
                println!("{addr:?}");
                if addr.is_ipv4() {
                    got_v4 = true;
                    if got_v6 == true {
                        break;
                    }
                }
                if addr.is_ipv6() {
                    got_v6 = true;
                    if got_v4 == true {
                        break;
                    }
                }
            }

            if !got_v4 {
                panic!("no v4 received");
            }

            if !got_v6 {
                panic!("no v6 received");
            }
        } => (),
    }
}

/*
#[tokio::test(flavor = "multi_thread")]
async fn sanity_v4() {
    let (recv_s, mut recv_r) = tokio::sync::mpsc::unbounded_channel();

    let _l = Socket::with_v4(
        std::net::Ipv4Addr::UNSPECIFIED,
        Some(MULTICAST_V4),
        PORT,
        Arc::new(move |res| {
            let _ = recv_s.send(res);
        }),
    )
    .await
    .unwrap();

    let c = Socket::with_v4(
        std::net::Ipv4Addr::UNSPECIFIED,
        None,
        0,
        Arc::new(|_| ()),
    )
    .await
    .unwrap();

    c.send(b"hello".to_vec(), (MULTICAST_V4, PORT).into())
        .await
        .unwrap();

    let (data, addr) =
        tokio::time::timeout(std::time::Duration::from_secs(10), recv_r.recv())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    println!("{:?}", addr);
    assert_eq!(b"hello", data.as_slice());
}

#[tokio::test(flavor = "multi_thread")]
async fn sanity_v6() {
    let (recv_s, mut recv_r) = tokio::sync::mpsc::unbounded_channel();

    let _l = Socket::with_v6(
        std::net::Ipv6Addr::UNSPECIFIED,
        Some(MULTICAST_V6),
        PORT,
        Arc::new(move |res| {
            let _ = recv_s.send(res);
        }),
    )
    .await
    .unwrap();

    let c = Socket::with_v6(
        std::net::Ipv6Addr::UNSPECIFIED,
        None,
        0,
        Arc::new(|_| ()),
    )
    .await
    .unwrap();

    c.send(b"hello".to_vec(), (MULTICAST_V6, PORT).into())
        .await
        .unwrap();

    let (data, addr) =
        tokio::time::timeout(std::time::Duration::from_secs(10), recv_r.recv())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    println!("{:?}", addr);
    assert_eq!(b"hello", data.as_slice());
}
*/
