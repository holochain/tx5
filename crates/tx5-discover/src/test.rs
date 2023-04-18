use super::*;

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
