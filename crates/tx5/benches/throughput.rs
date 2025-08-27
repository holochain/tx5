use criterion::{criterion_group, criterion_main, Criterion};
use std::io::{Error, Result};
use std::sync::Arc;
use tokio::sync::Mutex;
use tx5::*;

const DATA: &[u8] = &[0xdb; 4096];

struct Test {
    _server: sbd_server::SbdServer,
    cli_url1: PeerUrl,
    ep1: Endpoint,
    ep_rcv1: EndpointRecv,
    cli_url2: PeerUrl,
    ep2: Endpoint,
    ep_rcv2: EndpointRecv,
}

impl Test {
    pub async fn new() -> Self {
        let bind = vec![format!("127.0.0.1:0"), format!("[::1]:0")];

        let config = Arc::new(sbd_server::Config {
            bind,
            disable_rate_limiting: true,
            ..Default::default()
        });

        let server = sbd_server::SbdServer::new(config).await.unwrap();

        let sig_url = SigUrl::parse(&format!(
            "ws://{}",
            server.bind_addrs().get(0).unwrap()
        ))
        .unwrap();

        let config = Arc::new(Config::new().with_signal_allow_plain_text(true));

        let (ep1, mut ep_rcv1) = Endpoint::new(config.clone());
        ep1.listen(sig_url.clone()).await;
        let (ep2, mut ep_rcv2) = Endpoint::new(config);
        ep2.listen(sig_url).await;

        let (cli_url1, cli_url2) = tokio::join!(
            async {
                loop {
                    if let Some(EndpointEvent::ListeningAddressOpen {
                        local_url,
                    }) = ep_rcv1.recv().await
                    {
                        break local_url;
                    }
                }
            },
            async {
                loop {
                    if let Some(EndpointEvent::ListeningAddressOpen {
                        local_url,
                    }) = ep_rcv2.recv().await
                    {
                        break local_url;
                    }
                }
            },
        );

        ep1.send(cli_url2.clone(), b"hello".to_vec()).await.unwrap();
        match ep_rcv2.recv().await {
            Some(EndpointEvent::Connected { .. }) => (),
            oth => panic!("unexpected: {oth:?}"),
        }
        match ep_rcv2.recv().await {
            Some(EndpointEvent::Message { .. }) => (),
            oth => panic!("unexpected: {oth:?}"),
        }

        ep2.send(cli_url1.clone(), b"world".to_vec()).await.unwrap();
        match ep_rcv1.recv().await {
            Some(EndpointEvent::Connected { .. }) => (),
            oth => panic!("unexpected: {oth:?}"),
        }
        match ep_rcv1.recv().await {
            Some(EndpointEvent::Message { .. }) => (),
            oth => panic!("unexpected: {oth:?}"),
        }

        Self {
            _server: server,
            cli_url1,
            ep1,
            ep_rcv1,
            cli_url2,
            ep2,
            ep_rcv2,
        }
    }

    pub async fn test(&mut self) {
        let Test {
            cli_url1,
            ep1,
            ep_rcv1,
            cli_url2,
            ep2,
            ep_rcv2,
            ..
        } = self;

        let _ = tokio::try_join!(
            ep1.send(cli_url2.clone(), DATA.to_vec()),
            ep2.send(cli_url1.clone(), DATA.to_vec()),
            async { txerr(ep_rcv1.recv().await) },
            async { txerr(ep_rcv2.recv().await) },
        )
        .unwrap();
    }
}

fn txerr(v: Option<EndpointEvent>) -> Result<()> {
    match v {
        None => Err(Error::other("end")),
        _ => Ok(()),
    }
}

fn criterion_benchmark(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    let test = Arc::new(Mutex::new(rt.block_on(Test::new())));
    let test = &test;

    c.bench_function("throughput", |b| {
        b.to_async(&rt).iter(|| async move {
            test.lock().await.test().await;
        });
    });
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
