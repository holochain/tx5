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

pub struct TestSrv {
    server: sbd_server::SbdServer,
}

impl TestSrv {
    pub async fn new() -> Self {
        let config = Arc::new(sbd_server::Config {
            bind: vec!["127.0.0.1:0".to_string(), "[::1]:0".to_string()],
            ..Default::default()
        });

        let server = sbd_server::SbdServer::new(config).await.unwrap();

        Self { server }
    }

    pub async fn hub(&self) -> (Hub, HubRecv) {
        for addr in self.server.bind_addrs() {
            if let Ok(r) = Hub::new(
                &format!("ws://{addr}"),
                Arc::new(tx5_signal::SignalConfig {
                    listener: true,
                    allow_plain_text: true,
                    ..Default::default()
                }),
            )
            .await
            {
                return r;
            }
        }

        panic!()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn sanity() {
    init_tracing();

    let srv = TestSrv::new().await;

    let (hub1, _hubr1) = srv.hub().await;
    let pk1 = hub1.pub_key().clone();

    let (hub2, mut hubr2) = srv.hub().await;
    let pk2 = hub2.pub_key().clone();

    println!("connect");
    let (c1, mut r1) = hub1.connect(pk2).await.unwrap();
    println!("accept");
    let (c2, mut r2) = hubr2.accept().await.unwrap();

    assert_eq!(&pk1, c2.pub_key());

    println!("await ready");
    tokio::join!(c1.ready(), c2.ready());
    println!("ready");

    c1.send(b"hello".to_vec()).await.unwrap();
    assert_eq!(b"hello", r2.recv().await.unwrap().as_slice());

    c2.send(b"world".to_vec()).await.unwrap();
    assert_eq!(b"world", r1.recv().await.unwrap().as_slice());
}

#[tokio::test(flavor = "multi_thread")]
async fn framed_sanity() {
    init_tracing();

    let srv = TestSrv::new().await;

    let (hub1, _hubr1) = srv.hub().await;
    let pk1 = hub1.pub_key().clone();

    let (hub2, mut hubr2) = srv.hub().await;
    let pk2 = hub2.pub_key().clone();

    let ((c1, mut r1), (c2, mut r2)) = tokio::join!(
        async {
            let (c1, r1) = hub1.connect(pk2).await.unwrap();
            let limit =
                Arc::new(tokio::sync::Semaphore::new(512 * 1024 * 1024));
            let f = FramedConn::new(c1, r1, limit).await.unwrap();
            f
        },
        async {
            let (c2, r2) = hubr2.accept().await.unwrap();
            assert_eq!(&pk1, c2.pub_key());
            let limit =
                Arc::new(tokio::sync::Semaphore::new(512 * 1024 * 1024));
            let f = FramedConn::new(c2, r2, limit).await.unwrap();
            f
        },
    );

    c1.send(b"hello".to_vec()).await.unwrap();
    assert_eq!(b"hello", r2.recv().await.unwrap().as_slice());

    c2.send(b"world".to_vec()).await.unwrap();
    assert_eq!(b"world", r1.recv().await.unwrap().as_slice());
}

#[tokio::test(flavor = "multi_thread")]
async fn base_end_when_disconnected() {
    init_tracing();

    let srv = TestSrv::new().await;

    let (hub1, mut hubr1) = srv.hub().await;
    let pk1 = hub1.pub_key().clone();

    let (hub2, mut hubr2) = srv.hub().await;
    let pk2 = hub2.pub_key().clone();

    println!("connect");
    let (c1, mut r1) = hub1.connect(pk2.clone()).await.unwrap();
    println!("accept");
    let (c2, mut r2) = hubr2.accept().await.unwrap();

    println!("await ready");
    tokio::join!(c1.ready(), c2.ready());
    println!("ready");

    assert_eq!(&pk1, c2.pub_key());

    c1.send(b"hello".to_vec()).await.unwrap();
    assert_eq!(b"hello", r2.recv().await.unwrap().as_slice());

    c2.send(b"world".to_vec()).await.unwrap();
    assert_eq!(b"world", r1.recv().await.unwrap().as_slice());

    drop(srv);

    assert!(r1.recv().await.is_none());
    assert!(r2.recv().await.is_none());
    assert!(hubr1.accept().await.is_none());
    assert!(hubr2.accept().await.is_none());
    assert!(c1.send(b"hello".to_vec()).await.is_err());
    assert!(c2.send(b"hello".to_vec()).await.is_err());
    assert!(hub1.connect(pk2).await.is_err());
    assert!(hub2.connect(pk1).await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn framed_end_when_disconnected() {
    init_tracing();

    let srv = TestSrv::new().await;

    let (hub1, mut hubr1) = srv.hub().await;
    let pk1 = hub1.pub_key().clone();

    let (hub2, mut hubr2) = srv.hub().await;
    let pk2 = hub2.pub_key().clone();

    let ((c1, mut r1), (c2, mut r2)) = tokio::join!(
        async {
            let (c1, r2) = hub1.connect(pk2.clone()).await.unwrap();
            let limit =
                Arc::new(tokio::sync::Semaphore::new(512 * 1024 * 1024));
            let f = FramedConn::new(c1, r2, limit).await.unwrap();
            f
        },
        async {
            let (c2, r2) = hubr2.accept().await.unwrap();
            assert_eq!(&pk1, c2.pub_key());
            let limit =
                Arc::new(tokio::sync::Semaphore::new(512 * 1024 * 1024));
            let f = FramedConn::new(c2, r2, limit).await.unwrap();
            f
        },
    );

    c1.send(b"hello".to_vec()).await.unwrap();
    assert_eq!(b"hello", r2.recv().await.unwrap().as_slice());

    c2.send(b"world".to_vec()).await.unwrap();
    assert_eq!(b"world", r1.recv().await.unwrap().as_slice());

    drop(srv);

    assert!(r1.recv().await.is_none());
    assert!(r2.recv().await.is_none());
    assert!(hubr1.accept().await.is_none());
    assert!(hubr2.accept().await.is_none());
    assert!(c1.send(b"hello".to_vec()).await.is_err());
    assert!(c2.send(b"hello".to_vec()).await.is_err());
    assert!(hub1.connect(pk2).await.is_err());
    assert!(hub2.connect(pk1).await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn base_con_drop_disconnects() {
    init_tracing();

    let srv = TestSrv::new().await;

    let (hub1, _hubr1) = srv.hub().await;
    let pk1 = hub1.pub_key().clone();

    let (hub2, mut hubr2) = srv.hub().await;
    let pk2 = hub2.pub_key().clone();

    println!("connect");
    let (c1, mut r1) = hub1.connect(pk2.clone()).await.unwrap();
    println!("accept");
    let (c2, mut r2) = hubr2.accept().await.unwrap();

    println!("await ready");
    tokio::join!(c1.ready(), c2.ready());
    println!("ready");

    assert_eq!(&pk1, c2.pub_key());

    c1.send(b"hello".to_vec()).await.unwrap();
    assert_eq!(b"hello", r2.recv().await.unwrap().as_slice());

    c2.send(b"world".to_vec()).await.unwrap();
    assert_eq!(b"world", r1.recv().await.unwrap().as_slice());

    println!("drop c1");
    drop(c1);

    println!("check r1");
    assert!(r1.recv().await.is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn framed_con_drop_disconnects() {
    init_tracing();

    let srv = TestSrv::new().await;

    let (hub1, _hubr1) = srv.hub().await;
    let pk1 = hub1.pub_key().clone();

    let (hub2, mut hubr2) = srv.hub().await;
    let pk2 = hub2.pub_key().clone();

    let ((c1, mut r1), (c2, mut r2)) = tokio::join!(
        async {
            let (c1, r2) = hub1.connect(pk2.clone()).await.unwrap();
            let limit =
                Arc::new(tokio::sync::Semaphore::new(512 * 1024 * 1024));
            let f = FramedConn::new(c1, r2, limit).await.unwrap();
            f
        },
        async {
            let (c2, r2) = hubr2.accept().await.unwrap();
            assert_eq!(&pk1, c2.pub_key());
            let limit =
                Arc::new(tokio::sync::Semaphore::new(512 * 1024 * 1024));
            let f = FramedConn::new(c2, r2, limit).await.unwrap();
            f
        },
    );

    c1.send(b"hello".to_vec()).await.unwrap();
    assert_eq!(b"hello", r2.recv().await.unwrap().as_slice());

    c2.send(b"world".to_vec()).await.unwrap();
    assert_eq!(b"world", r1.recv().await.unwrap().as_slice());

    println!("drop c1");
    drop(c1);

    println!("check r1");
    assert!(r1.recv().await.is_none());
}
