use crate::*;

pub struct Sig {
    cmd_send: tokio::sync::mpsc::UnboundedSender<SigCmd>,
}

#[derive(Debug)]
enum SigCmd {
    Shutdown,
    Demo,
    SendOffer(state::PeerId, String),
    SendAnswer(state::PeerId, String),
    SendICE(state::PeerId, String),
}

impl Drop for Sig {
    fn drop(&mut self) {
        let _ = self.cmd_send.send(SigCmd::Shutdown);
    }
}

impl Sig {
    pub async fn spawn_to_core(
        config: Arc<config::Config>,
        lair: lair::Lair,
        core: core::Core,
    ) -> Result<()> {
        let (cmd_send, cmd_recv) = tokio::sync::mpsc::unbounded_channel();

        core.sig(Sig {
            cmd_send: cmd_send.clone(),
        });

        tokio::task::spawn(sig_task(core, config, lair, cmd_send, cmd_recv));

        Ok(())
    }

    pub fn send_offer(&self, id: state::PeerId, offer: String) {
        let _ = self.cmd_send.send(SigCmd::SendOffer(id, offer));
    }

    pub fn send_answer(&self, id: state::PeerId, answer: String) {
        let _ = self.cmd_send.send(SigCmd::SendAnswer(id, answer));
    }

    pub fn send_ice(&self, id: state::PeerId, ice: String) {
        let _ = self.cmd_send.send(SigCmd::SendICE(id, ice));
    }
}

async fn sig_task(
    core: core::Core,
    config: Arc<config::Config>,
    lair: lair::Lair,
    cmd_send: tokio::sync::mpsc::UnboundedSender<SigCmd>,
    mut cmd_recv: tokio::sync::mpsc::UnboundedReceiver<SigCmd>,
) {
    let demo_abort = tokio::task::spawn(async move {
        loop {
            use rand::Rng;
            let secs = rand::thread_rng().gen_range(30..60);
            tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
            if cmd_send.send(SigCmd::Demo).is_err() {
                break;
            }
        }
    });

    struct KillDemo(tokio::task::JoinHandle<()>);

    impl Drop for KillDemo {
        fn drop(&mut self) {
            self.0.abort();
        }
    }

    let _kill_demo = KillDemo(demo_abort);

    let mut backoff = std::time::Duration::from_secs(1);
    let mut signal_url_iter = config.signal.iter();

    'sig_top: loop {
        let signal_url = match signal_url_iter.next() {
            Some(url) => url,
            None => {
                signal_url_iter = config.signal.iter();
                match signal_url_iter.next() {
                    Some(url) => url,
                    None => panic!("EmptySignalConfig"),
                }
            }
        };

        tracing::info!(%signal_url, "Connecting to signal server...");

        let recv_cb = {
            let core = core.clone();
            Box::new(move |msg| {
                core.sig_msg(msg);
            })
        };

        let mut sig_cli = match tx4_signal::cli::Cli::builder()
            .with_recv_cb(recv_cb)
            .with_lair_client(lair.lair_client.clone())
            .with_lair_tag(config.lair_tag.clone())
            .with_url(signal_url.clone())
            .build()
            .await
        {
            Ok(sig_cli) => sig_cli,
            Err(err) => {
                let mut secs = backoff.as_secs() * 2;
                if secs > 60 {
                    secs = 60;
                }
                backoff = std::time::Duration::from_secs(secs);
                tracing::warn!(?err, ?backoff);
                tokio::time::sleep(backoff).await;
                continue 'sig_top;
            }
        };

        backoff = std::time::Duration::from_secs(2);

        core.ice(sig_cli.ice_servers().clone());

        let signal_addr = sig_cli.local_addr().clone();
        if let Err(err) = sig_cli.demo().await {
            tracing::warn!(?err);
            continue 'sig_top;
        }

        tracing::info!(%signal_addr);
        core.addr(signal_addr);

        while let Some(cmd) = cmd_recv.recv().await {
            match cmd {
                SigCmd::Shutdown => break,
                SigCmd::Demo => {
                    if let Err(err) = sig_cli.demo().await {
                        tracing::warn!(?err);
                        continue 'sig_top;
                    }
                }
                SigCmd::SendOffer(id, offer) => {
                    let offer: serde_json::Value =
                        match serde_json::from_str(&offer) {
                            Err(err) => {
                                tracing::warn!(?err);
                                continue; // just the local loop
                            }
                            Ok(offer) => offer,
                        };

                    if let Err(err) =
                        sig_cli.offer(&id.rem_id, &id.rem_pk, &offer).await
                    {
                        tracing::warn!(?err);
                        continue 'sig_top;
                    }
                }
                SigCmd::SendAnswer(id, answer) => {
                    let answer: serde_json::Value =
                        match serde_json::from_str(&answer) {
                            Err(err) => {
                                tracing::warn!(?err);
                                continue; // just the local loop
                            }
                            Ok(answer) => answer,
                        };

                    if let Err(err) =
                        sig_cli.answer(&id.rem_id, &id.rem_pk, &answer).await
                    {
                        tracing::warn!(?err);
                        continue 'sig_top;
                    }
                }
                SigCmd::SendICE(id, ice) => {
                    let ice: serde_json::Value =
                        match serde_json::from_str(&ice) {
                            Err(err) => {
                                tracing::warn!(?err);
                                continue; // just the local loop
                            }
                            Ok(ice) => ice,
                        };

                    if let Err(err) =
                        sig_cli.ice(&id.rem_id, &id.rem_pk, &ice).await
                    {
                        tracing::warn!(?err);
                        continue 'sig_top;
                    }
                }
            }
        }
    }
}
