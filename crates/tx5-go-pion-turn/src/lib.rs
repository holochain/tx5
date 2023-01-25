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

const EXEC_BYTES: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/tx5-go-pion-turn"));

pub use tx5_core::{Error, ErrorExt, Id, Result};

use once_cell::sync::Lazy;

static EXEC: Lazy<tempfile::TempPath> = Lazy::new(|| {
    use std::io::Write;

    let mut file =
        tempfile::NamedTempFile::new().expect("failed to open temp file");

    file.write_all(EXEC_BYTES)
        .expect("filed to write executable bytes");
    file.flush().expect("failed to flush executable bytes");

    let mut perms = file
        .as_file()
        .metadata()
        .expect("failed to read metadata")
        .permissions();
    perms.set_readonly(true);

    #[cfg(not(windows))]
    std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);

    file.as_file_mut()
        .set_permissions(perms)
        .expect("failed to set metadata");

    file.into_temp_path()
});

/// Rust process wrapper around tx5-go-pion-turn executable.
pub struct Tx5TurnServer {
    proc: tokio::process::Child,
}

impl Tx5TurnServer {
    /// Start up a new TURN server.
    pub async fn new() -> Result<Self> {
        let mut cmd = tokio::process::Command::new(EXEC.as_os_str());
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
