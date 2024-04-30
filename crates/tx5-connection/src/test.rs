use super::*;

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

    pub async fn hub(&self) -> Tx5ConnectionHub {
        for addr in self.server.bind_addrs() {
            if let Ok(sig) = tx5_signal::SignalConnection::connect(
                &format!("ws://{addr}"),
                Arc::new(tx5_signal::SignalConfig {
                    listener: true,
                    allow_plain_text: true,
                    ..Default::default()
                }),
            )
            .await
            {
                return Tx5ConnectionHub::new(sig);
            }
        }

        panic!()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn sanity() {
    let srv = TestSrv::new().await;

    let hub1 = srv.hub().await;
    let pk1 = hub1.pub_key().clone();

    let hub2 = srv.hub().await;
    let pk2 = hub2.pub_key().clone();

    println!("connect");
    let c1 = hub1.connect(pk2).await.unwrap();
    println!("accept");
    let c2 = hub2.accept().await.unwrap();

    assert_eq!(&pk1, c2.pub_key());

    println!("await ready");
    tokio::join!(c1.ready(), c2.ready());
    println!("ready");

    c1.send(b"hello".to_vec()).await.unwrap();
    assert_eq!(b"hello", c2.recv().await.unwrap().as_slice());

    c2.send(b"world".to_vec()).await.unwrap();
    assert_eq!(b"world", c1.recv().await.unwrap().as_slice());
}
