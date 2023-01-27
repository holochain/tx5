#![deny(missing_docs)]
#![deny(warnings)]
#![deny(unsafe_code)]

//! Rust process wrapper around tx5-go-pion-turn executable.
//!
//! [![Project](https://img.shields.io/badge/project-holochain-blue.svg?style=flat-square)](http://holochain.org/)
//! [![Forum](https://img.shields.io/badge/chat-forum%2eholochain%2enet-blue.svg?style=flat-square)](https://forum.holochain.org)
//! [![Chat](https://img.shields.io/badge/chat-chat%2eholochain%2enet-blue.svg?style=flat-square)](https://chat.holochain.org)
//!
//! [![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://opensource.org/licenses/MIT)
//! [![License: Apache-2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://www.apache.org/licenses/LICENSE-2.0)

/// Re-exported dependencies.
pub mod deps {
    pub use tx5_core;
    pub use tx5_core::deps::*;
}

const EXE_BYTES: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/tx5-go-pion-turn"));

include!(concat!(env!("OUT_DIR"), "/exe_hash.rs"));

pub use tx5_core::{Error, ErrorExt, Id, Result};

use once_cell::sync::Lazy;

// keep the file handle open to mitigate replacements on some os
static EXE: Lazy<(std::path::PathBuf, std::fs::File)> = Lazy::new(|| {
    let mut path =
        dirs::data_local_dir().expect("failed to determine data dir");

    #[cfg(windows)]
    let ext = ".exe";
    #[cfg(not(windows))]
    let ext = "";

    path.push(format!("tx5-go-pion-turn-{}{}", EXE_HASH, ext));

    let mut opts = std::fs::OpenOptions::new();

    opts.write(true);
    opts.create_new(true);

    #[cfg(not(windows))]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o700);

    if let Ok(mut file) = opts.open(&path) {
        use std::io::Write;

        file.write_all(EXE_BYTES)
            .expect("failed to write executable bytes");
        file.flush().expect("failed to flush executable bytes");
    }

    if let Ok(mut file) = std::fs::OpenOptions::new().read(true).open(&path) {
        use std::io::Read;

        let mut data = Vec::new();
        file.read_to_end(&mut data)
            .expect("failed to read executable");

        use sha2::Digest;
        let mut hasher = sha2::Sha256::new();
        hasher.update(data);
        let hash =
            base64::encode_config(hasher.finalize(), base64::URL_SAFE_NO_PAD);

        assert_eq!(EXE_HASH, hash);

        (path, file)
    } else {
        panic!("invalid executable");
    }
});

/// Rust process wrapper around tx5-go-pion-turn executable.
pub struct Tx5TurnServer {
    proc: tokio::process::Child,
}

impl Tx5TurnServer {
    /// Start up a new TURN server.
    pub async fn new() -> Result<Self> {
        let mut cmd = tokio::process::Command::new(EXE.0.as_os_str());
        cmd.kill_on_drop(true);

        println!("ABOUT TO SPAWN: {:?}", cmd);

        let proc = cmd.spawn()?;

        Ok(Self { proc })
    }

    /// Stop and clean up the TURN server sub-process.
    /// Note, a drop will attempt to clean up the process, but to be sure,
    /// use this function.
    pub async fn stop(mut self) -> Result<()> {
        self.proc.kill().await?;
        self.proc.wait().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn sanity() {
        let srv = Tx5TurnServer::new().await.unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(300)).await;

        srv.stop().await.unwrap();
    }
}
