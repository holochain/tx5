//! Connect to a signal server, and run diagnostics on the TURN info.
//!
//! ## Results if run vs xirsys setup:
//!
//! ```text
//! SIG_URL: ws://127.0.0.1:8443/
//! SIG_ICE: PeerConnectionConfig {
//!     ice_servers: [
//!         IceServer {
//!             urls: [
//!                 "stun:ws-turn6.xirsys.com:80",
//!                 "turn:ws-turn6.xirsys.com:80",
//!                 "turn:ws-turn6.xirsys.com:80?transport=tcp",
//!                 "turns:ws-turn6.xirsys.com:443?transport=tcp",
//!             ],
//!             username: Some(
//!                 "<snip>",
//!             ),
//!             credential: Some(
//!                 "<snip>",
//!             ),
//!         },
//!     ],
//! }
//! TURN_CHECK: TurnCheck {
//!     host: "ws-turn6.xirsys.com",
//!     user: "<snip>",
//!     cred: "<snip>",
//!     stun_port: 80,
//!     udp_port: 80,
//!     tcp_plain_port: 80,
//!     tcp_tls_port: 443,
//! }
//! CHECK_STUN: PeerConnectionConfig { ice_servers: [IceServer { urls: ["stun:ws-turn6.xirsys.com:80"], username: Some("<snip>"), credential: Some("<snip>") }] }
//! ICE: [
//!     (
//!         "srflx",
//!         "<snip>",
//!     ),
//! ]
//! OKAY: CHECK_STUN
//! CHECK_UDP: PeerConnectionConfig { ice_servers: [IceServer { urls: ["turn:ws-turn6.xirsys.com:80"], username: Some("<snip>"), credential: Some("<snip>") }] }
//! ICE: [
//!     (
//!         "srflx",
//!         "<snip>",
//!     ),
//!     (
//!         "relay",
//!         "<snip>",
//!     ),
//! ]
//! OKAY: CHECK_UDP
//! CHECK_TCP_PLAIN: PeerConnectionConfig { ice_servers: [IceServer { urls: ["turn:ws-turn6.xirsys.com:80?transport=tcp"], username: Some("<snip>"), credential: Some("<snip>") }] }
//! ICE: [
//!     (
//!         "srflx",
//!         "<snip>",
//!     ),
//!     (
//!         "relay",
//!         "<snip>",
//!     ),
//! ]
//! OKAY: CHECK_TCP_PLAIN
//! CHECK_TCP_TLS: PeerConnectionConfig { ice_servers: [IceServer { urls: ["turns:ws-turn6.xirsys.com:80?transport=tcp"], username: Some("<snip>"), credential: Some("<snip>") }] }
//! ICE: [
//!     (
//!         "srflx",
//!         "<snip>",
//!     ),
//!     (
//!         "relay",
//!         "<snip>",
//!     ),
//! ]
//! OKAY: CHECK_TCP_TLS
//! ```
//!
//! Results if run vs holochain setup:
//!
//! ```text
//! SIG_URL: wss://signal.holo.host/
//! SIG_ICE: PeerConnectionConfig {
//!     ice_servers: [
//!         IceServer {
//!             urls: [
//!                 "turn:turn.holo.host:443",
//!             ],
//!             username: Some(
//!                 "test",
//!             ),
//!             credential: Some(
//!                 "test",
//!             ),
//!         },
//!     ],
//! }
//! WARN: stun not found (e.g. stun:my.stun:80) - this could be okay if your turn server is handling stun traffic
//! WARN: udp plain port is 443, recommended to be 80
//! WARN: tcp plain not found (e.g. turn:my.turn:80?transport=tcp) - this could be okay if you want to jump straight to tcp tls
//! ERROR: tpc tls not found (e.g. turns:my.turn:443?transport=tcp) - this is the most firewall pass-able transport and is recommended to be enabled
//! TURN_CHECK: TurnCheck {
//!     host: "turn.holo.host",
//!     user: "test",
//!     cred: "test",
//!     stun_port: 443,
//!     udp_port: 443,
//!     tcp_plain_port: 443,
//!     tcp_tls_port: 443,
//! }
//! CHECK_STUN: PeerConnectionConfig { ice_servers: [IceServer { urls: ["stun:turn.holo.host:443"], username: Some("test"), credential: Some("test") }] }
//! ICE: [
//!     (
//!         "srflx",
//!         "<snip>",
//!     ),
//! ]
//! OKAY: CHECK_STUN
//! CHECK_UDP: PeerConnectionConfig { ice_servers: [IceServer { urls: ["turn:turn.holo.host:443"], username: Some("test"), credential: Some("test") }] }
//! ICE: [
//!     (
//!         "srflx",
//!         "<snip>",
//!     ),
//!     (
//!         "relay",
//!         "<snip>",
//!     ),
//! ]
//! OKAY: CHECK_UDP
//! CHECK_TCP_PLAIN: PeerConnectionConfig { ice_servers: [IceServer { urls: ["turn:turn.holo.host:443?transport=tcp"], username: Some("test"), credential: Some("test") }] }
//! ICE: [
//!     (
//!         "srflx",
//!         "<snip>",
//!     ),
//!     (
//!         "relay",
//!         "<snip>",
//!     ),
//! ]
//! OKAY: CHECK_TCP_PLAIN
//! CHECK_TCP_TLS: PeerConnectionConfig { ice_servers: [IceServer { urls: ["turns:turn.holo.host:443?transport=tcp"], username: Some("test"), credential: Some("test") }] }
//! ICE: [
//!     (
//!         "srflx",
//!         "<snip>",
//!     ),
//!     (
//!         "relay",
//!         "<snip>",
//!     ),
//! ]
//! OKAY: CHECK_TCP_TLS
//! ```

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

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    init_tracing();

    let args = std::env::args().collect::<Vec<_>>();
    let sig_url = args.get(1).expect("required signal_url");
    let sig_url = url::Url::parse(sig_url).unwrap();
    println!("SIG_URL: {sig_url}");

    let (sig_cli, _sig_rcv) = tx5_signal::Cli::builder()
        .with_url(sig_url)
        .build()
        .await
        .expect("expect can build tx5_signal::Cli");

    let ice: tx5_go_pion::PeerConnectionConfig = serde_json::from_str(
        &serde_json::to_string(&sig_cli.ice_servers()).unwrap(),
    )
    .unwrap();

    println!("SIG_ICE: {ice:#?}");

    let turn_check = check_ice(&ice);

    println!("TURN_CHECK: {turn_check:#?}");

    stun_check(&turn_check).await;
    turn_udp_check(&turn_check).await;
    turn_tcp_plain_check(&turn_check).await;
    turn_tcp_tls_check(&turn_check).await;
}

#[derive(Debug)]
struct TurnCheck {
    pub host: String,
    pub user: String,
    pub cred: String,
    pub stun_port: u16,
    pub udp_port: u16,
    pub tcp_plain_port: u16,
    pub tcp_tls_port: u16,
}

fn check_ice(ice: &tx5_go_pion::PeerConnectionConfig) -> TurnCheck {
    let mut found_stun = false;
    let mut found_udp_plain = false;
    let mut found_tcp_plain = false;
    let mut found_tcp_tls = false;

    let mut turn_check = None;

    for server in ice.ice_servers.iter() {
        let user = server.username.as_ref().unwrap();
        let cred = server.credential.as_ref().unwrap();

        for url in server.urls.iter() {
            let url = url::Url::parse(url).unwrap();

            let scheme = url.scheme().to_string();
            let transport = url
                .query_pairs()
                .find(|(k, _)| k == "transport")
                .map(|(_, v)| v.to_string())
                .unwrap_or_else(|| "udp".to_string());

            let url =
                url::Url::parse(&format!("fake://{}", url.path())).unwrap();

            let host = url.host_str().unwrap().to_string();
            let port = url.port().unwrap();

            match (scheme.as_str(), transport.as_str()) {
                ("stun", _) => found_stun = true,
                ("turn", "udp") => found_udp_plain = true,
                ("turn", "tcp") => found_tcp_plain = true,
                ("turns", "tcp") => found_tcp_tls = true,
                oth => panic!("unexpected scheme/transport: {oth:?}"),
            }

            if turn_check.is_none() {
                turn_check = Some(TurnCheck {
                    host,
                    user: user.clone(),
                    cred: cred.clone(),
                    stun_port: port,
                    udp_port: port,
                    tcp_plain_port: port,
                    tcp_tls_port: port,
                });
            } else {
                let r = turn_check.as_mut().unwrap();
                if r.host != host || &r.user != user || &r.cred != cred {
                    panic!(
                        "doctor doesn't support multiple host/user/cred yet"
                    );
                }
                match (scheme.as_str(), transport.as_str()) {
                    ("stun", _) => r.stun_port = port,
                    ("turn", "udp") => r.udp_port = port,
                    ("turn", "tcp") => r.tcp_plain_port = port,
                    ("turns", "tcp") => r.tcp_tls_port = port,
                    _ => (),
                }
            }
        }
    }

    let turn_check = turn_check.unwrap();

    if found_stun {
        if turn_check.stun_port != 80 {
            println!(
                "WARN: stun port is {}, recommended to be 80",
                turn_check.stun_port
            );
        }
    } else {
        println!("WARN: stun not found (e.g. stun:my.stun:80) - this could be okay if your turn server is handling stun traffic");
    }

    if found_udp_plain {
        if turn_check.udp_port != 80 {
            println!(
                "WARN: udp plain port is {}, recommended to be 80",
                turn_check.udp_port
            );
        }
    } else {
        println!("ERROR: plain udp transport not found (e.g. turn:my.turn:80) - this is the lowest overhead transport and is recommended to be enabled");
    }

    if found_tcp_plain {
        if turn_check.tcp_plain_port != 80 {
            println!(
                "WARN: tcp plain port is {}, recommended to be 80",
                turn_check.tcp_plain_port
            );
        }
    } else {
        println!("WARN: tcp plain not found (e.g. turn:my.turn:80?transport=tcp) - this could be okay if you want to jump straight to tcp tls");
    }

    if found_tcp_tls {
        if turn_check.tcp_tls_port != 443 {
            println!(
                "WARN: tcp tls port is {}, recommended to be 443",
                turn_check.tcp_tls_port
            );
        }
    } else {
        println!("ERROR: tpc tls not found (e.g. turns:my.turn:443?transport=tcp) - this is the most firewall pass-able transport and is recommended to be enabled");
    }

    turn_check
}

async fn gather_ice(
    config: tx5_go_pion::PeerConnectionConfig,
) -> Vec<(String, String)> {
    let mut out_ice = Vec::new();
    {
        let out_ice = &mut out_ice;
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            async move {
                let (con, mut r) = tx5_go_pion::PeerConnection::new(
                    config,
                    std::sync::Arc::new(tokio::sync::Semaphore::new(
                        usize::MAX >> 3,
                    )),
                )
                .await
                .unwrap();

                let _dc = con
                    .create_data_channel(
                        tx5_go_pion::DataChannelConfig::default(),
                    )
                    .await
                    .unwrap();
                let offer = con
                    .create_offer(tx5_go_pion::OfferConfig::default())
                    .await
                    .unwrap();
                con.set_local_description(offer).await.unwrap();

                #[derive(serde::Deserialize)]
                struct C {
                    pub candidate: String,
                }

                while let Some((evt, _p)) = r.recv().await {
                    if let tx5_go_pion::PeerConnectionEvent::ICECandidate(
                        mut ice,
                    ) = evt
                    {
                        let c: C = ice.as_json().unwrap();
                        let mut typ = "unknown".to_string();
                        {
                            let mut t = c.candidate.split("typ");
                            if let Some(_) = t.next() {
                                if let Some(r) = t.next() {
                                    let mut t = r.split(" ");
                                    if let Some(_) = t.next() {
                                        if let Some(r) = t.next() {
                                            typ = r.to_string();
                                        }
                                    }
                                }
                            }
                        }
                        if typ != "unknown" && typ != "host" {
                            out_ice.push((typ, c.candidate));
                        }
                    }
                }
            },
        )
        .await;
    }
    out_ice
}

async fn stun_check(check: &TurnCheck) {
    let config = tx5_go_pion::PeerConnectionConfig {
        ice_servers: vec![tx5_go_pion::IceServer {
            urls: vec![format!("stun:{}:{}", check.host, check.stun_port)],
            username: Some(check.user.clone()),
            credential: Some(check.cred.clone()),
        }],
    };
    println!("CHECK_STUN: {config:?}");
    let ice = gather_ice(config).await;
    println!("ICE: {ice:#?}");
    let mut found_srflx = false;
    for (typ, _) in ice {
        if typ == "srflx" {
            found_srflx = true;
        }
    }
    if found_srflx {
        println!("OKAY: CHECK_STUN");
    } else {
        println!("ERROR: unable to get srflx stun response within 10 seconds");
    }
}

async fn turn_udp_check(check: &TurnCheck) {
    turn_check(
        "CHECK_UDP",
        tx5_go_pion::PeerConnectionConfig {
            ice_servers: vec![tx5_go_pion::IceServer {
                urls: vec![format!("turn:{}:{}", check.host, check.stun_port)],
                username: Some(check.user.clone()),
                credential: Some(check.cred.clone()),
            }],
        },
    )
    .await;
}

async fn turn_tcp_plain_check(check: &TurnCheck) {
    turn_check(
        "CHECK_TCP_PLAIN",
        tx5_go_pion::PeerConnectionConfig {
            ice_servers: vec![tx5_go_pion::IceServer {
                urls: vec![format!(
                    "turn:{}:{}?transport=tcp",
                    check.host, check.stun_port
                )],
                username: Some(check.user.clone()),
                credential: Some(check.cred.clone()),
            }],
        },
    )
    .await;
}

async fn turn_tcp_tls_check(check: &TurnCheck) {
    turn_check(
        "CHECK_TCP_TLS",
        tx5_go_pion::PeerConnectionConfig {
            ice_servers: vec![tx5_go_pion::IceServer {
                urls: vec![format!(
                    "turns:{}:{}?transport=tcp",
                    check.host, check.stun_port
                )],
                username: Some(check.user.clone()),
                credential: Some(check.cred.clone()),
            }],
        },
    )
    .await;
}

async fn turn_check(tag: &str, config: tx5_go_pion::PeerConnectionConfig) {
    println!("{tag}: {config:?}");
    let ice = gather_ice(config).await;
    println!("ICE: {ice:#?}");
    let mut found_relay = false;
    for (typ, _) in ice {
        if typ == "relay" {
            found_relay = true;
        }
    }
    if found_relay {
        println!("OKAY: {tag}");
    } else {
        println!("ERROR: unable to get relay response within 10 seconds");
    }
}
