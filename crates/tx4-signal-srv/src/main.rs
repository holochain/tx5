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
use tx4_signal_core::*;

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

jsdoc! { Binding {
    [
        (), local_interface,
        std::net::SocketAddr, ("0.0.0.0:0".parse().unwrap()),
        hb1, "#localInterface", "`ip:port` of local interface to bind",
    ],
    [
        (), wan_host,
        String, (Default::default()),
        hb2, "#wanHost", "wan host to publish in url",
    ],
    [
        (), wan_port,
        u16, (Default::default()),
        hb3, "#wanPort", "wan port to publish in url",
    ],
    [
        (), enabled,
        bool, (true),
        hb4, "#enabled", "`true` if this interface should be bound",
    ],
    [
        (#[serde(default, skip_deserialization)]), notes,
        Vec<String>, (Default::default()),
        hb5, "#notes", "unparsed user information about this interface",
    ],
}}

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
        (), binding_list,
        Vec<Binding>, (Default::default()),
        hc1, "#bindingList", "list of local interfaces to bind",
    ],
    [
        (), tls_cert_der_b64,
        String, (Default::default()),
        hc2, "#tlsCertDerB64", "base64 DER TLS certificate",
    ],
    [
        (), tls_cert_pk_der_b64,
        String, (Default::default()),
        hc3, "#tlsCertPkDerB64", "base64 DER TLS certificate private key",
    ],
    [
        (), ice_servers,
        serde_json::Value, (serde_json::from_str(ICE_SERVERS).unwrap()),
        hc4, "#iceServers", "webrtc configuration to broadcast",
    ],
    [
        (), demo,
        bool, (false),
        hc5, "#demo", "enable demo broadcasting as a stand-in for bootstrapping",
    ],
}}

/// Main executable entrypoint.
#[tokio::main(flavor = "multi_thread")]
pub async fn main() {
    let subscriber = tracing_subscriber::FmtSubscriber::builder()
        .with_env_filter(tracing_subscriber::filter::EnvFilter::from_default_env())
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

    let srv = srv_builder.build().await?;

    println!(
        "#tx4-signal-srv LISTEN#\n{}\n#tx4-signal-srv START#",
        srv.local_addr(),
    );

    futures::future::pending().await
}

#[doc(hidden)]
async fn read_config(opt: Opt) -> Result<tx4_signal_core::srv::SrvBuilder> {
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use tokio::io::AsyncReadExt;

    let config = opt.config.as_ref().unwrap();

    let mut file = match tokio::fs::OpenOptions::new().read(true).open(config).await {
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
        binding_list,
        tls_cert_der_b64,
        tls_cert_pk_der_b64,
        ice_servers,
        demo,
        ..
    } = conf;

    let tls_cert_der = match base64::decode(&tls_cert_der_b64) {
        Err(err) => {
            return Err(Error::err(format!(
                "Failed to parse config file {:?}: {:?}",
                config, err,
            )))
        }
        Ok(cert) => tls::TlsCertDer(cert.into_boxed_slice()),
    };

    let tls_cert_pk_der = match base64::decode(&tls_cert_pk_der_b64) {
        Err(err) => {
            return Err(Error::err(format!(
                "Failed to parse config file {:?}: {:?}",
                config, err,
            )))
        }
        Ok(pk) => tls::TlsPkDer(pk.into_boxed_slice()),
    };

    let tls = match tls::TlsConfigBuilder::default()
        .with_cert(tls_cert_der, tls_cert_pk_der)
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

    let mut srv_builder = tx4_signal_core::srv::SrvBuilder::default()
        .with_tls(tls)
        .with_ice_servers(serde_json::to_string(&ice_servers).unwrap())
        .with_allow_demo(demo);

    for binding in binding_list {
        if !binding.enabled {
            continue;
        }
        srv_builder.add_bind(binding.local_interface, binding.wan_host, binding.wan_port);
    }

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

    let (cert_r, cert_r_pk) = tls::gen_tls_cert_pair().unwrap();
    let cert = base64::encode(&cert_r.0);
    let cert_pk = base64::encode(&cert_r_pk.0);

    let mut config = Config {
        tls_cert_der_b64: cert,
        tls_cert_pk_der_b64: cert_pk,
        ..Default::default()
    };

    let mut found_v4 = false;
    let mut found_v6 = false;

    use rand::Rng;
    let port = rand::thread_rng().gen_range(32768..60999);
    for iface in get_if_addrs::get_if_addrs()? {
        let ip = iface.ip();
        let is_loopback = ip.is_loopback();
        let is_global = ip.ext_is_global();
        let mut enabled = !is_loopback;
        let mut notes = Vec::new();
        notes.push(format!("iface: {}", iface.name));
        if is_loopback {
            notes.push("loopback: disabled".into());
        }
        if is_global {
            notes.push("global: directly addressable".into());
        }
        if !is_loopback && !is_global {
            notes.push("private: configure port-forwarding, update host".into());
        }
        match ip {
            std::net::IpAddr::V4(ip) => {
                if !is_loopback {
                    if found_v4 {
                        enabled = false;
                        notes.push("multiple v4: disabled".into());
                    } else {
                        found_v4 = true;
                        notes.push("first_v4: enabled".into());
                    }
                }
                config.binding_list.push(Binding {
                    local_interface: std::net::SocketAddr::V4(std::net::SocketAddrV4::new(
                        ip, port,
                    )),
                    wan_host: ip.to_string(),
                    wan_port: port,
                    enabled,
                    notes,
                    ..Default::default()
                });
            }
            std::net::IpAddr::V6(ip) => {
                if !is_loopback {
                    if found_v6 {
                        enabled = false;
                        notes.push("multiple v6: disabled".into());
                    } else {
                        found_v6 = true;
                        notes.push("first_v6: enabled".into());
                    }
                }
                config.binding_list.push(Binding {
                    local_interface: std::net::SocketAddr::V6(std::net::SocketAddrV6::new(
                        ip, port, 0, 0,
                    )),
                    wan_host: format!("[{}]", ip),
                    wan_port: port,
                    enabled,
                    notes,
                    ..Default::default()
                });
            }
        }
    }

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

#[doc(hidden)]
trait IpAddrExt {
    fn ext_is_global(&self) -> bool;
}

impl IpAddrExt for std::net::IpAddr {
    #[inline]
    fn ext_is_global(&self) -> bool {
        match self {
            std::net::IpAddr::V4(a) => a.ext_is_global(),
            std::net::IpAddr::V6(a) => a.ext_is_global(),
        }
    }
}

impl IpAddrExt for std::net::Ipv4Addr {
    #[inline]
    fn ext_is_global(&self) -> bool {
        if u32::from_be_bytes(self.octets()) == 0xc0000009
            || u32::from_be_bytes(self.octets()) == 0xc000000a
        {
            return true;
        }
        !self.is_private()
            && !self.is_loopback()
            && !self.is_link_local()
            && !self.is_broadcast()
            && !self.is_documentation()
            // is_shared()
            && !(self.octets()[0] == 100 && (self.octets()[1] & 0b1100_0000 == 0b0100_0000))
            && !(self.octets()[0] == 192 && self.octets()[1] == 0 && self.octets()[2] == 0)
            // is_reserved()
            && !(self.octets()[0] & 240 == 240 && !self.is_broadcast())
            // is_benchmarking()
            && !(self.octets()[0] == 198 && (self.octets()[1] & 0xfe) == 18)
            && self.octets()[0] != 0
    }
}

impl IpAddrExt for std::net::Ipv6Addr {
    #[inline]
    fn ext_is_global(&self) -> bool {
        !self.is_multicast()
            && !self.is_loopback()
            //&& !self.is_unicast_link_local()
            && !((self.segments()[0] & 0xffc0) == 0xfe80)
            //&& !self.is_unique_local()
            && !((self.segments()[0] & 0xfe00) == 0xfc00)
            && !self.is_unspecified()
            //&& !self.is_documentation()
            && !((self.segments()[0] == 0x2001) && (self.segments()[1] == 0xdb8))
    }
}
