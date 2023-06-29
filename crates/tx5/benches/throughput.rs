use criterion::{criterion_group, criterion_main, Criterion};
use std::sync::Arc;
use tokio::sync::Mutex;
use tx5::{actor::ManyRcv, *};

const DATA: &[u8] = &[0xdb; 4096];

struct Test {
    cli_url1: Tx5Url,
    ep1: Ep,
    ep_rcv1: ManyRcv<EpEvt>,
    cli_url2: Tx5Url,
    ep2: Ep,
    ep_rcv2: ManyRcv<EpEvt>,
}

impl Test {
    pub async fn new() -> Self {
        let mut srv_config = tx5_signal_srv::Config::default();
        srv_config.port = 0;
        srv_config.demo = true;

        let (srv_driver, addr_list, _) =
            tx5_signal_srv::exec_tx5_signal_srv(srv_config).unwrap();
        tokio::task::spawn(srv_driver);

        let sig_port = addr_list.get(0).unwrap().port();

        let sig_url =
            Tx5Url::new(format!("ws://localhost:{sig_port}")).unwrap();

        let (ep1, mut ep_rcv1) = Ep::new().await.unwrap();
        let cli_url1 = ep1.listen(sig_url.clone()).await.unwrap();
        let (ep2, mut ep_rcv2) = Ep::new().await.unwrap();
        let cli_url2 = ep2.listen(sig_url).await.unwrap();

        ep1.send(cli_url2.clone(), &b"hello"[..]).await.unwrap();
        match ep_rcv2.recv().await {
            Some(Ok(EpEvt::Connected { .. })) => (),
            oth => panic!("unexpected: {oth:?}"),
        }
        match ep_rcv2.recv().await {
            Some(Ok(EpEvt::Data { .. })) => (),
            oth => panic!("unexpected: {oth:?}"),
        }

        ep2.send(cli_url1.clone(), &b"world"[..]).await.unwrap();
        match ep_rcv1.recv().await {
            Some(Ok(EpEvt::Connected { .. })) => (),
            oth => panic!("unexpected: {oth:?}"),
        }
        match ep_rcv1.recv().await {
            Some(Ok(EpEvt::Data { .. })) => (),
            oth => panic!("unexpected: {oth:?}"),
        }

        Self {
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
        } = self;

        let _ = tokio::try_join!(
            ep1.send(cli_url2.clone(), DATA),
            ep2.send(cli_url1.clone(), DATA),
            async { ep_rcv1.recv().await.unwrap() },
            async { ep_rcv2.recv().await.unwrap() },
        )
        .unwrap();
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
