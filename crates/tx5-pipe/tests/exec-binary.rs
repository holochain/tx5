#[tokio::test(flavor = "multi_thread")]
async fn exec_binary() {
    let exec_path = escargot::CargoBuild::new()
        .bin("tx5-pipe")
        .run()
        .unwrap()
        .path()
        .to_owned();

    println!("tx5-pipe path: {:?}", exec_path);

    let (driver, boot_addr, shutdown) =
        kitsune_p2p_bootstrap::run(([127, 0, 0, 1], 0), vec![])
            .await
            .unwrap();
    tokio::task::spawn(driver);
    let boot_addr = format!("http://{boot_addr}");
    println!("boot: {boot_addr}");

    struct OnDrop(Option<kitsune_p2p_bootstrap::BootstrapShutdown>);
    impl Drop for OnDrop {
        fn drop(&mut self) {
            if let Some(s) = self.0.take() {
                s();
            }
        }
    }
    let _on_drop = OnDrop(Some(shutdown));

    let mut config = tx5_signal_srv::Config::default();
    config.port = 0;
    config.demo = false;
    config.interfaces = "127.0.0.1".to_string();

    let (driver, mut sig_addr, _) =
        tx5_signal_srv::exec_tx5_signal_srv(config).unwrap();
    let sig_addr = sig_addr.remove(0);
    tokio::task::spawn(driver);
    let sig_addr = format!("ws://{sig_addr}");
    println!("sig: {sig_addr}");

    struct H;

    impl tx5_pipe::client::Tx5PipeClientHandler for H {
        fn recv(&self, _rem_url: String, _data: Box<dyn bytes::Buf + Send>) {}
    }

    let (cli, mut child) =
        tx5_pipe::client::Tx5PipeClient::spawn_child(H, exec_path)
            .await
            .unwrap();

    let cli_url = cli.sig_reg(sig_addr).await.unwrap();
    println!("sig cli url: {cli_url}");

    cli.boot_reg(
        boot_addr.clone(),
        [0; 32],
        Some(cli_url),
        Box::new(std::io::Cursor::new(b"hello".to_vec())),
    )
    .await
    .unwrap();

    let boot_query_resp = cli.boot_query(boot_addr, [0; 32]).await.unwrap();
    println!("boot_query_resp: {boot_query_resp:#?}");

    drop(cli);
    child.kill().await.unwrap();
}
