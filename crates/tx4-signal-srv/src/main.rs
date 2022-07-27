#![deny(missing_docs)]
#![deny(warnings)]
#![deny(unsafe_code)]
// these are copied out of std-lib nightly... rather leave them as-is
#![allow(clippy::nonminimal_bool)]

//! Holochain webrtc signal server.
//!
//! [![Project](https://img.shields.io/badge/project-holochain-blue.svg?style=flat-square)](http://holochain.org/)
//! [![Forum](https://img.shields.io/badge/chat-forum%2eholochain%2enet-blue.svg?style=flat-square)](https://forum.holochain.org)
//! [![Chat](https://img.shields.io/badge/chat-chat%2eholochain%2enet-blue.svg?style=flat-square)](https://chat.holochain.org)
//!
//! [![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://opensource.org/licenses/MIT)
//! [![License: Apache-2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://www.apache.org/licenses/LICENSE-2.0)

#![doc = include_str!("docs/srv_help.md")]

use clap::Parser;
use tx4_signal::*;

#[derive(Debug, Parser)]
#[clap(
    name = "tx4-signal-srv",
    version,
    about = "Holochain Webrtc Signal Server"
)]
#[doc(hidden)]
struct Opt {
    /// Initialize a new tx4-signal-srv.json configuration file
    /// (as specified by --config).
    /// Will abort if it already exists.
    #[clap(short, long)]
    init: bool,

    /// Configuration file to use for running the tx4-signal-srv.
    /// Defaults to `$user_config_dir_path$/tx4-signal-srv.json`.
    #[clap(short, long)]
    config: Option<std::path::PathBuf>,
}

#[doc(hidden)]
macro_rules! jsdoc {
    ($n:ident {$(
        [($($rne:tt)*), $rn:ident, $rt:ty, ($rd:expr), $dn:ident, $jn:literal, $d:literal,],
    )*}) => {
        /// tx4-signal-srv config
        #[derive(serde::Serialize, serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        #[non_exhaustive]
        #[doc(hidden)]
        pub struct $n {$(
            #[serde(default, skip_deserializing)]
            #[serde(rename = $jn)]
            $dn: &'static str,
            $rn: $rt,
        )*}

        $(
            #[allow(non_upper_case_globals)]
            #[doc(hidden)]
            const $dn: &'static str = $d;
        )*

        impl Default for $n {
            fn default() -> Self {
                Self {$(
                    $dn,
                    $rn: $rd,
                )*}
            }
        }
    };
}

#[doc(hidden)]
const ICE_SERVERS: &str = r#"[
    {
      "urls": ["stun:openrelay.metered.ca:80"]
    },
    {
      "urls": ["turn:openrelay.metered.ca:80"],
      "username": "openrelayproject",
      "credential": "openrelayproject"
    },
    {
      "urls": ["turn:openrelay.metered.ca:443"],
      "username": "openrelayproject",
      "credential": "openrelayproject"
    },
    {
      "urls": ["turn:openrelay.metered.ca:443?transport=tcp"],
      "username": "openrelayproject",
      "credential": "openrelayproject"
    }
]"#;

jsdoc! { Config {
    [
        (), port,
        u16, (8443),
        hc1, "#port", "port to bind",
    ],
    [
        (), tls_cert_pem,
        tls::Pem, (tls::Pem(Default::default())),
        hc2, "#tlsCertPem", "PEM encoded TLS certificate",
    ],
    [
        (), tls_cert_pk_pem,
        tls::Pem, (tls::Pem(Default::default())),
        hc3, "#tlsCertPkPem", "PEM encoded TLS certificate private key",
    ],
    [
        (), ice_servers,
        serde_json::Value, (serde_json::from_str(ICE_SERVERS).unwrap()),
        hc4, "#iceServers", "webrtc configuration to broadcast",
    ],
    [
        (), demo,
        bool, (true),
        hc5, "#demo", "enable demo broadcasting as a stand-in for bootstrapping",
    ],
}}

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
        eprintln!("{}", err);
        std::process::exit(1);
    }
}

#[doc(hidden)]
async fn main_err() -> Result<()> {
    let mut opt = Opt::parse();
    if opt.config.is_none() {
        let mut config = dirs::config_dir().unwrap_or_else(|| {
            let mut config = std::path::PathBuf::new();
            config.push(".");
            config
        });
        config.push("tx4-signal-srv.json");
        opt.config = Some(config);
    }

    if opt.init {
        return run_init(opt).await;
    }

    let srv_builder = read_config(opt).await?;

    let _srv = srv_builder.build().await?;

    println!("#tx4-signal-srv START#");

    futures::future::pending().await
}

#[doc(hidden)]
async fn read_config(opt: Opt) -> Result<tx4_signal::srv::SrvBuilder> {
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use tokio::io::AsyncReadExt;

    let config = opt.config.as_ref().unwrap();

    let mut file =
        match tokio::fs::OpenOptions::new().read(true).open(config).await {
            Err(err) => {
                return Err(Error::err(format!(
                    "Failed to open config file {:?}: {:?}",
                    config, err,
                )))
            }
            Ok(file) => file,
        };

    let perms = match file.metadata().await {
        Err(err) => {
            return Err(Error::err(format!(
                "Failed to load config file metadata {:?}: {:?}",
                config, err
            )))
        }
        Ok(perms) => perms.permissions(),
    };

    if !perms.readonly() {
        return Err(Error::err(format!(
            "Refusing to run with writable config file {:?}",
            config
        )));
    }

    #[cfg(unix)]
    {
        let mode = perms.mode() & 0o777;
        if mode != 0o400 {
            return Err(Error::err(format!(
                "Refusing to run with config file not set to mode 0o400 {:?} 0o{:o}",
                config, mode,
            )));
        }
    }

    let mut conf = String::new();
    if let Err(err) = file.read_to_string(&mut conf).await {
        return Err(Error::err(format!(
            "Failed to read config file {:?}: {:?}",
            config, err,
        )));
    }

    let conf: Config = match serde_json::from_str(&conf) {
        Err(err) => {
            return Err(Error::err(format!(
                "Failed to parse config file {:?}: {:?}",
                config, err,
            )))
        }
        Ok(res) => res,
    };

    let Config {
        port,
        tls_cert_pem,
        tls_cert_pk_pem,
        ice_servers,
        demo,
        ..
    } = conf;

    let tls = match tls::TlsConfigBuilder::default()
        .with_cert(tls_cert_pem, tls_cert_pk_pem)
        .build()
    {
        Err(err) => {
            return Err(Error::err(format!(
                "Failed to build TlsConfig from config file {:?}: {:?}",
                config, err,
            )))
        }
        Ok(tls) => tls,
    };

    let srv_builder = tx4_signal::srv::SrvBuilder::default()
        .with_port(port)
        .with_tls(tls)
        .with_ice_servers(serde_json::to_string(&ice_servers).unwrap())
        .with_allow_demo(demo);

    Ok(srv_builder)
}

#[doc(hidden)]
async fn run_init(opt: Opt) -> Result<()> {
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use tokio::io::AsyncWriteExt;

    let config_fn = opt.config.as_ref().unwrap();

    let mut file = tokio::fs::OpenOptions::new();
    file.create_new(true);
    file.write(true);
    let mut file = match file.open(config_fn).await {
        Err(err) => {
            return Err(Error::err(format!(
                "Failed to create config file {:?}: {:?}",
                config_fn, err,
            )))
        }
        Ok(file) => file,
    };

    let (cert, cert_pk) = tls::tls_self_signed()?;

    let config = Config {
        tls_cert_pem: cert,
        tls_cert_pk_pem: cert_pk,
        ..Default::default()
    };

    let mut config = serde_json::to_string_pretty(&config).unwrap();
    config.push('\n');

    if let Err(err) = file.write_all(config.as_bytes()).await {
        return Err(Error::err(format!(
            "Failed to initialize config file {:?}: {:?}",
            config_fn, err
        )));
    };

    let mut perms = match file.metadata().await {
        Err(err) => {
            return Err(Error::err(format!(
                "Failed to load config file metadata {:?}: {:?}",
                config_fn, err,
            )))
        }
        Ok(perms) => perms.permissions(),
    };
    perms.set_readonly(true);

    #[cfg(unix)]
    perms.set_mode(0o400);

    if let Err(err) = file.set_permissions(perms).await {
        return Err(Error::err(format!(
            "Failed to set config file permissions {:?}: {:?}",
            config_fn, err,
        )));
    }

    if let Err(err) = file.shutdown().await {
        return Err(Error::err(format!(
            "Failed to flush/close config file: {:?}",
            err
        )));
    }

    println!("# hc-rtc-sig-srv wrote {:?} #", config_fn);

    Ok(())
}
