use crate::deps::*;
use crate::*;

/// Tx5 signal server execution configuration.
#[derive(Debug, Parser)]
#[clap(
    name = "tx5-signal-srv",
    version,
    about = "Holochain Webrtc Signal Server"
)]
#[non_exhaustive]
pub struct Opt {
    /// Initialize a new tx5-signal-srv.json configuration file
    /// (as specified by --config).
    /// Will abort if it already exists.
    #[clap(short, long)]
    pub init: bool,

    /// Run the signal server, generating a config file if one
    /// does not already exist. Exclusive with "init" option.
    #[clap(long)]
    pub run_with_init_if_needed: bool,

    /// Configuration file to use for running the tx5-signal-srv.
    /// Defaults to `$user_config_dir_path$/tx5-signal-srv.json`.
    #[clap(short, long)]
    pub config: Option<std::path::PathBuf>,
}

#[doc(hidden)]
macro_rules! jsdoc {
    ($n:ident {$(
        [($($rne:tt)*), $rn:ident, $rt:ty, ($rd:expr), $dn:ident, $jn:literal, $d:literal,],
    )*}) => {
        /// Tx5-signal-srv config.
        #[derive(Debug, serde::Serialize, serde::Deserialize)]
        #[serde(rename_all = "camelCase", crate = "crate::deps::serde")]
        #[non_exhaustive]
        pub struct $n {$(
            #[serde(default, skip_deserializing)]
            #[serde(rename = $jn)]
            $dn: &'static str,
            #[allow(missing_docs)]
            pub $rn: $rt,
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
    /*
    [
        (), tls_cert_pem,
        tls::Pem, (tls::Pem(Default::default())),
        hc2, "#tlsCertPem", "PEM encoded TLS certificate",
    ],
    [
        (), tls_cert_priv_pem,
        tls::Pem, (tls::Pem(Default::default())),
        hc3, "#tlsCertPrivPem", "PEM encoded TLS certificate private key",
    ],
    */
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

/// Result type from the [config_per_opt] function.
pub enum ConfigPerOpt {
    /// [config_per_opt] wrote the config file to the given path.
    ConfigWritten(std::path::PathBuf),

    /// [config_per_opt] read and parsed the given [Config].
    ConfigLoaded(Config),
}

/// If [Opt] specifies "init", write an example configuration file,
/// otherwise read and parse the config file.
pub async fn config_per_opt(mut opt: Opt) -> Result<ConfigPerOpt> {
    if opt.config.is_none() {
        let mut config = dirs::config_dir().unwrap_or_else(|| {
            let mut config = std::path::PathBuf::new();
            config.push(".");
            config
        });
        config.push("tx5-signal-srv.json");
        opt.config = Some(config);
    }

    let config_path = opt.config.as_ref().unwrap().to_owned();

    if opt.init && opt.run_with_init_if_needed {
        panic!("--init and --run-with-init-if-needed cannot both be specified");
    } else if opt.init {
        write_example_config(&config_path).await?;
        return Ok(ConfigPerOpt::ConfigWritten(config_path));
    } else if opt.run_with_init_if_needed {
        let _ = write_example_config(&config_path).await;
    }

    Ok(ConfigPerOpt::ConfigLoaded(read_config(&config_path).await?))
}

/// Read and parse a tx5-signal-srv.json configuration file.
pub async fn read_config(config_path: &std::path::Path) -> Result<Config> {
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use tokio::io::AsyncReadExt;

    let mut file = match tokio::fs::OpenOptions::new()
        .read(true)
        .open(config_path)
        .await
    {
        Err(err) => {
            return Err(Error::err(format!(
                "Failed to open config file {:?}: {:?}",
                config_path, err,
            )))
        }
        Ok(file) => file,
    };

    let perms = match file.metadata().await {
        Err(err) => {
            return Err(Error::err(format!(
                "Failed to load config file metadata {:?}: {:?}",
                config_path, err
            )))
        }
        Ok(perms) => perms.permissions(),
    };

    if !perms.readonly() {
        return Err(Error::err(format!(
            "Refusing to run with writable config file {:?}",
            config_path
        )));
    }

    #[cfg(unix)]
    {
        let mode = perms.mode() & 0o777;
        if mode != 0o400 {
            return Err(Error::err(format!(
                "Refusing to run with config file not set to mode 0o400 {:?} 0o{:o}",
                config_path, mode,
            )));
        }
    }

    let mut conf = String::new();
    if let Err(err) = file.read_to_string(&mut conf).await {
        return Err(Error::err(format!(
            "Failed to read config file {:?}: {:?}",
            config_path, err,
        )));
    }

    let conf: Config = match serde_json::from_str(&conf) {
        Err(err) => {
            return Err(Error::err(format!(
                "Failed to parse config file {:?}: {:?}",
                config_path, err,
            )))
        }
        Ok(res) => res,
    };

    /*
    let Config {
        port,
        tls_cert_pem,
        tls_cert_priv_pem,
        ice_servers,
        demo,
        ..
    } = conf;

    let tls = match tls::TlsConfigBuilder::default()
        .with_cert(tls_cert_pem, tls_cert_priv_pem)
        .build()
    {
        Err(err) => {
            return Err(Error::err(format!(
                "Failed to build TlsConfig from config file {:?}: {:?}",
                config_path, err,
            )))
        }
        Ok(tls) => tls,
    };
    */

    Ok(conf)
}

/// Encode and write an example tx5-signal-srv.json configuration file.
pub async fn write_example_config(config_path: &std::path::Path) -> Result<()> {
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use tokio::io::AsyncWriteExt;

    let mut file = tokio::fs::OpenOptions::new();
    file.create_new(true);
    file.write(true);
    let mut file = match file.open(config_path).await {
        Err(err) => {
            return Err(Error::err(format!(
                "Failed to create config file {:?}: {:?}",
                config_path, err,
            )))
        }
        Ok(file) => file,
    };

    //let (cert, cert_priv) = tls::tls_self_signed()?;

    let config = Config::default();
    // let mut config = Config::default();
    // config.tls_cert_pem = cert;
    // config.tls_cert_priv_pem = cert_priv;

    let mut config = serde_json::to_string_pretty(&config).unwrap();
    config.push('\n');

    if let Err(err) = file.write_all(config.as_bytes()).await {
        return Err(Error::err(format!(
            "Failed to initialize config file {:?}: {:?}",
            config_path, err
        )));
    };

    let mut perms = match file.metadata().await {
        Err(err) => {
            return Err(Error::err(format!(
                "Failed to load config file metadata {:?}: {:?}",
                config_path, err,
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
            config_path, err,
        )));
    }

    if let Err(err) = file.shutdown().await {
        return Err(Error::err(format!(
            "Failed to flush/close config file: {:?}",
            err
        )));
    }

    Ok(())
}
