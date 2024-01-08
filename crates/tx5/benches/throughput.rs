use criterion::{criterion_group, criterion_main, Criterion};
use std::sync::Arc;
use tokio::sync::Mutex;
use tx5::*;
use tx5_core::EventRecv;

const DATA: &[u8] = &[0xdb; 4096];

struct Test {
    _sig_srv_hnd: tx5_signal_srv::SrvHnd,
    cli_url1: PeerUrl,
    ep1: Ep3,
    ep_rcv1: EventRecv<Ep3Event>,
    cli_url2: PeerUrl,
    ep2: Ep3,
    ep_rcv2: EventRecv<Ep3Event>,
}

impl Test {
    pub async fn new() -> Self {
        let mut srv_config = tx5_signal_srv::Config::default();
        srv_config.port = 0;
        srv_config.demo = true;

        let (_sig_srv_hnd, addr_list, _) =
            tx5_signal_srv::exec_tx5_signal_srv(srv_config)
                .await
                .unwrap();

        let sig_port = addr_list.get(0).unwrap().port();

        let sig_url =
            Tx5Url::new(format!("ws://localhost:{sig_port}")).unwrap();

        let config = Arc::new(Config3::default());

        let (ep1, mut ep_rcv1) = Ep3::new(config.clone()).await;
        let cli_url1 = ep1.listen(sig_url.clone()).await.unwrap();
        let (ep2, mut ep_rcv2) = Ep3::new(config).await;
        let cli_url2 = ep2.listen(sig_url).await.unwrap();

        ep1.send(
            cli_url2.clone(),
            vec![BackBuf::from_slice(b"hello").unwrap()],
        )
        .await
        .unwrap();
        match ep_rcv2.recv().await {
            Some(Ep3Event::Connected { .. }) => (),
            oth => panic!("unexpected: {oth:?}"),
        }
        match ep_rcv2.recv().await {
            Some(Ep3Event::Message { .. }) => (),
            oth => panic!("unexpected: {oth:?}"),
        }

        ep2.send(
            cli_url1.clone(),
            vec![BackBuf::from_slice(b"world").unwrap()],
        )
        .await
        .unwrap();
        match ep_rcv1.recv().await {
            Some(Ep3Event::Connected { .. }) => (),
            oth => panic!("unexpected: {oth:?}"),
        }
        match ep_rcv1.recv().await {
            Some(Ep3Event::Message { .. }) => (),
            oth => panic!("unexpected: {oth:?}"),
        }

        Self {
            _sig_srv_hnd,
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
            _sig_srv_hnd,
            cli_url1,
            ep1,
            ep_rcv1,
            cli_url2,
            ep2,
            ep_rcv2,
        } = self;

        let _ = tokio::try_join!(
            ep1.send(
                cli_url2.clone(),
                vec![BackBuf::from_slice(DATA).unwrap()]
            ),
            ep2.send(
                cli_url1.clone(),
                vec![BackBuf::from_slice(DATA).unwrap()]
            ),
            async { txerr(ep_rcv1.recv().await) },
            async { txerr(ep_rcv2.recv().await) },
        )
        .unwrap();
    }
}

fn txerr(v: Option<Ep3Event>) -> Result<()> {
    match v {
        None => Err(Error::id("end")),
        Some(Ep3Event::Error(err)) => Err(err.into()),
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
