#![deny(missing_docs)]
#![deny(unsafe_code)]
#![doc = tx5_core::__doc_header!()]
//! # tx5-go-pion-turn
//!
//! Rust process wrapper around tx5-go-pion-turn executable.

/// Re-exported dependencies.
pub mod deps {
    pub use tx5_core;
    pub use tx5_core::deps::*;
}

const EXE_BYTES: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/tx5-go-pion-turn"));

include!(concat!(env!("OUT_DIR"), "/exe_hash.rs"));

pub use tx5_core::{Error, ErrorExt, Result};

use once_cell::sync::Lazy;

// keep the file handle open to mitigate replacements on some oses
static EXE: Lazy<tx5_core::file_check::FileCheck> = Lazy::new(|| {
    #[cfg(windows)]
    let ext = ".exe";
    #[cfg(not(windows))]
    let ext = "";

    match tx5_core::file_check::file_check(
        EXE_BYTES,
        EXE_HASH,
        "tx5-go-pion-turn",
        ext,
    ) {
        Err(err) => panic!("failed to write turn exe: {err:?}"),
        Ok(exe) => exe,
    }
});

/// Rust process wrapper around tx5-go-pion-turn executable.
pub struct Tx5TurnServer {
    proc: tokio::process::Child,
}

impl Tx5TurnServer {
    /// Start up a new TURN server.
    pub async fn new(
        public_ip: std::net::IpAddr,
        bind_port: u16,
        users: Vec<(String, String)>,
        realm: String,
    ) -> Result<(String, Self)> {
        let mut cmd = tokio::process::Command::new(EXE.path());
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        cmd.kill_on_drop(true);
        cmd.arg("-public-ip");
        cmd.arg(public_ip.to_string());
        cmd.arg("-port");
        cmd.arg(format!("{bind_port}"));
        cmd.arg("-realm");
        cmd.arg(realm);
        cmd.arg("-users");
        cmd.arg(
            users
                .into_iter()
                .map(|(u, p)| format!("{u}={p}"))
                .collect::<Vec<_>>()
                .join(","),
        );

        tracing::info!("ABOUT TO SPAWN: {cmd:?}");

        let mut proc = cmd.spawn()?;
        drop(proc.stdin.take());
        let mut stderr = proc.stderr.take().unwrap();

        let mut output = Vec::new();
        let mut buf = [0; 4096];
        loop {
            use tokio::io::AsyncReadExt;

            let len = stderr.read(&mut buf).await?;
            if len == 0 {
                return Err(Error::id("BrokenPipe"));
            }
            output.extend_from_slice(&buf[0..len]);

            let s = String::from_utf8_lossy(&output);
            let mut i = s.split("#ICE#(");
            if i.next().is_some() {
                if let Some(s) = i.next() {
                    if s.contains(")#") {
                        let mut i = s.split(")#");
                        if let Some(s) = i.next() {
                            return Ok((s.to_string(), Self { proc }));
                        }
                    }
                }
            }
        }
    }

    /// Stop and clean up the TURN server sub-process.
    /// Note, a drop will attempt to clean up the process, but to be sure,
    /// use this function.
    pub async fn stop(mut self) -> Result<()> {
        // note, if the process already ended, kill may return an error.
        let _ = self.proc.kill().await;
        self.proc.wait().await?;
        Ok(())
    }
}

/// Construct an ephemeral turn server on an available port designed for
/// unit testing.
pub async fn test_turn_server() -> Result<(String, Tx5TurnServer)> {
    let mut addr = None;

    for iface in if_addrs::get_if_addrs()? {
        if iface.is_loopback() {
            continue;
        }
        if iface.ip().is_ipv6() {
            continue;
        }
        addr = Some(iface.ip());
        break;
    }

    if addr.is_none() {
        return Err(Error::id("NoLocalIp"));
    }

    let (turn, srv) = Tx5TurnServer::new(
        addr.unwrap(),
        0,
        vec![("test".into(), "test".into())],
        "holo.host".into(),
    )
    .await?;

    let turn = format!("{{\"urls\":[\"{turn}\"],\"username\":\"test\",\"credential\":\"test\"}}");

    Ok((turn, srv))
}

#[cfg(test)]
mod tests {
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

    #[tokio::test(flavor = "multi_thread")]
    async fn sanity() {
        init_tracing();

        let (ice, srv) = test_turn_server().await.unwrap();

        println!("{}", ice);

        tokio::time::sleep(std::time::Duration::from_millis(300)).await;

        srv.stop().await.unwrap();
    }
}
