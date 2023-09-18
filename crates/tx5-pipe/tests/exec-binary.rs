use once_cell::sync::Lazy;
use std::sync::{Arc, Mutex};

static BIN: Lazy<std::path::PathBuf> = Lazy::new(|| {
    escargot::CargoBuild::new()
        .bin("tx5-pipe")
        .run()
        .unwrap()
        .path()
        .to_owned()
});

struct OnDrop {
    boot_shutdown: Option<kitsune_p2p_bootstrap::BootstrapShutdown>,
}

impl Drop for OnDrop {
    fn drop(&mut self) {
        if let Some(s) = self.boot_shutdown.take() {
            s();
        }
    }
}

struct Setup {
    on_drop: OnDrop,
    boot_addr: String,
    sig_addr: String,
}

impl Setup {
    pub async fn new() -> Self {
        let (driver, boot_addr, shutdown) =
            kitsune_p2p_bootstrap::run(([127, 0, 0, 1], 0), vec![])
                .await
                .unwrap();
        tokio::task::spawn(driver);
        let boot_addr = format!("http://{boot_addr}");

        let on_drop = OnDrop {
            boot_shutdown: Some(shutdown),
        };

        let mut config = tx5_signal_srv::Config::default();
        config.port = 0;
        config.demo = false;
        config.interfaces = "127.0.0.1".to_string();

        let (driver, mut sig_addr, _) =
            tx5_signal_srv::exec_tx5_signal_srv(config).unwrap();
        let sig_addr = sig_addr.remove(0);
        tokio::task::spawn(driver);
        let sig_addr = format!("ws://{sig_addr}");

        Self {
            on_drop,
            boot_addr,
            sig_addr,
        }
    }
}

async fn pipe_quit(data: &str) -> String {
    use tokio::io::AsyncWriteExt;

    let mut child = tokio::process::Command::new(&*BIN)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();

    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(data.as_bytes()).await.unwrap();
    drop(stdin);

    let output = child.wait_with_output().await.unwrap();

    if !output.status.success() {
        panic!("error: {}", String::from_utf8_lossy(&output.stderr));
    }

    String::from_utf8_lossy(&output.stdout).to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn binary_pipe_quit_hash() {
    let result = pipe_quit("hash h testing\nquit\n").await;
    assert!(result
        .contains("@hash_ok h mTl_8yrjSLi2U21cIT80PX6f3qoQ6KI6n5CrIaFlhWU"));
}

#[tokio::test(flavor = "multi_thread")]
async fn binary_pipe_quit_sig_reg() {
    let Setup {
        on_drop: _on_drop,
        boot_addr: _,
        sig_addr,
    } = Setup::new().await;

    let result = pipe_quit(&format!(
        r#"sig_reg r {}
quit
"#,
        sig_addr,
    ))
    .await;
    assert!(result.contains("@sig_reg_ok r"));
}

#[tokio::test(flavor = "multi_thread")]
async fn binary_pipe_quit_boot_reg_query() {
    let Setup {
        on_drop: _on_drop,
        boot_addr,
        sig_addr: _,
    } = Setup::new().await;

    let result = pipe_quit(
        &format!(
            r#"boot_reg r {} mTl_8yrjSLi2U21cIT80PX6f3qoQ6KI6n5CrIaFlhWU wss://yada ""
quit
"#,
            boot_addr,
        ),
    ).await;
    assert!(result.contains("@boot_reg_ok r"));

    let result = pipe_quit(&format!(
        r#"boot_query q {} mTl_8yrjSLi2U21cIT80PX6f3qoQ6KI6n5CrIaFlhWU
quit
"#,
        boot_addr,
    ))
    .await;
    assert!(result.contains("@boot_query_resp q"));
    assert!(result.contains("@boot_query_ok q"));
}

#[derive(Clone)]
struct TestHandler(pub Arc<Mutex<Vec<(String, String)>>>);

impl TestHandler {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(Vec::new())))
    }
}

impl tx5_pipe_control::Tx5PipeControlHandler for TestHandler {
    fn recv(&self, rem_url: String, data: Box<dyn bytes::Buf + Send>) {
        self.0.lock().unwrap().push((rem_url, into_string(data)));
    }
}

fn into_string<B: bytes::Buf>(mut b: B) -> String {
    let mut out = Vec::with_capacity(b.remaining());
    while b.has_remaining() {
        let c = b.chunk();
        out.extend_from_slice(c);
        b.advance(c.len());
    }
    String::from_utf8(out).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn binary_private_convo() {
    let Setup {
        on_drop: _on_drop,
        boot_addr: _,
        sig_addr,
    } = Setup::new().await;

    let hnd1 = TestHandler::new();

    let (cli1, mut child1) =
        tx5_pipe_control::Tx5PipeControl::spawn_child(hnd1.clone(), &*BIN)
            .await
            .unwrap();

    let cli1_url = cli1.sig_reg(sig_addr.clone()).await.unwrap();
    println!("sig cli1 url: {cli1_url}");

    let hnd2 = TestHandler::new();

    let (cli2, mut child2) =
        tx5_pipe_control::Tx5PipeControl::spawn_child(hnd2.clone(), &*BIN)
            .await
            .unwrap();

    let cli2_url = cli2.sig_reg(sig_addr).await.unwrap();
    println!("sig cli2 url: {cli2_url}");

    cli1.send(cli2_url, Box::new(std::io::Cursor::new(b"hello")))
        .await
        .unwrap();
    cli2.send(cli1_url, Box::new(std::io::Cursor::new(b"world")))
        .await
        .unwrap();

    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;

            let c1 = hnd1.0.lock().unwrap().len();
            let c2 = hnd2.0.lock().unwrap().len();

            if c1 > 0 && c2 > 0 {
                break;
            }
        }
    })
    .await
    .unwrap();

    let m1 = hnd1.0.lock().unwrap().remove(0);
    let m2 = hnd2.0.lock().unwrap().remove(0);

    assert_eq!("world", m1.1);
    assert_eq!("hello", m2.1);

    cli1.quit();
    cli2.quit();
    drop(cli1);
    drop(cli2);
    child1.wait().await.unwrap();
    child2.wait().await.unwrap();
}
