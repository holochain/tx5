#![deny(missing_docs)]
#![deny(warnings)]
#![deny(unsafe_code)]

//! Holochain webrtc signal server.
//!
//! [![Project](https://img.shields.io/badge/project-holochain-blue.svg?style=flat-square)](http://holochain.org/)
//! [![Forum](https://img.shields.io/badge/chat-forum%2eholochain%2enet-blue.svg?style=flat-square)](https://forum.holochain.org)
//! [![Chat](https://img.shields.io/badge/chat-chat%2eholochain%2enet-blue.svg?style=flat-square)](https://chat.holochain.org)
//!
//! [![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://opensource.org/licenses/MIT)
//! [![License: Apache-2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://www.apache.org/licenses/LICENSE-2.0)

#![doc = include_str!("../docs/srv_help.md")]

use clap::Parser;
use tx5_signal_srv::*;

/// Main executable entrypoint.
#[tokio::main(flavor = "multi_thread")]
pub async fn main() {
    let subscriber = tracing_subscriber::FmtSubscriber::builder()
        .with_env_filter(
            tracing_subscriber::filter::EnvFilter::from_default_env(),
        )
        .with_file(true)
        .with_line_number(true)
        .finish();
    tracing::subscriber::set_global_default(subscriber).unwrap();

    if let Err(err) = main_err().await {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

#[doc(hidden)]
async fn main_err() -> Result<()> {
    let opt = Opt::parse();

    let config = match config_per_opt(opt).await? {
        ConfigPerOpt::ConfigWritten(path) => {
            println!("# tx5-signal-srv WROTE CONFIG\n{path:?}");
            return Ok(());
        }
        ConfigPerOpt::ConfigLoaded(config) => config,
    };

    let (_hnd, addr_list, err_list) = exec_tx5_signal_srv(config).await?;

    for err in err_list {
        println!("# tx5-signal-srv ERR {err:?}");
    }

    for addr in addr_list {
        println!("# tx5-signal-srv ADDR ws://{addr}");
    }

    println!("# tx5-signal-srv START");

    futures::future::pending().await
}
