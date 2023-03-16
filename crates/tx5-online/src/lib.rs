#![deny(missing_docs)]
#![deny(unsafe_code)]
#![deny(warnings)]
#![doc = tx5_core::__doc_header!()]
//! # tx5-online
//!
//! Holochain WebRTC p2p communication ecosystem online connectivity events.

use once_cell::sync::Lazy;
use trust_dns_resolver::config::*;
use trust_dns_resolver::error::*;
use trust_dns_resolver::*;

/// An online status event.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OnlineEvent {
    /// The machine is now likely online (we were able to reach the network).
    Online,

    /// The machine is now likely offline (we were unable to reach the network).
    Offline,
}

impl std::fmt::Display for OnlineEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

/// A receiver handle to the online event emitter.
#[derive(Clone)]
pub struct OnlineReceiver(tokio::sync::watch::Receiver<OnlineEvent>);

impl Default for OnlineReceiver {
    fn default() -> Self {
        RCV.clone()
    }
}

impl OnlineReceiver {
    /// Get a new receiver handle to the online event emitter.
    pub fn new() -> Self {
        Self::default()
    }

    /// Get the current online status.
    pub fn status(&mut self) -> OnlineEvent {
        // the sender is never destroyed, so this can never error
        while self.0.has_changed().unwrap() {
            self.0.borrow_and_update();
        }
        *self.0.borrow_and_update()
    }

    /// Await the next online status event.
    pub async fn recv(&mut self) -> OnlineEvent {
        // the sender is never destroyed, so this can never error
        let _ = self.0.changed().await;
        *self.0.borrow_and_update()
    }
}

static RCV: Lazy<OnlineReceiver> = Lazy::new(|| {
    let (snd, rcv) = tokio::sync::watch::channel(OnlineEvent::Offline);

    tokio::task::spawn(async move {
        loop {
            let status = check_status().await;

            snd.send_if_modified(move |s| {
                if *s == status {
                    false
                } else {
                    *s = status;
                    true
                }
            });

            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    });

    OnlineReceiver(rcv)
});

async fn check_status() -> OnlineEvent {
    let mut servers = Vec::new();
    servers.append(&mut NameServerConfigGroup::cloudflare().into_inner());
    servers.append(&mut NameServerConfigGroup::google().into_inner());
    servers.append(&mut NameServerConfigGroup::quad9().into_inner());

    rand::seq::SliceRandom::shuffle(&mut servers[..], &mut rand::thread_rng());

    let conf = ResolverConfig::from_parts(None, Vec::new(), servers);

    let mut opts = ResolverOpts::default();
    opts.server_ordering_strategy = ServerOrderingStrategy::UserProvidedOrder;
    opts.cache_size = 0;
    opts.use_hosts_file = false;

    let r = TokioAsyncResolver::tokio(conf, opts).unwrap();

    const TLD: [&str; 5] = [".com.", ".net.", ".org.", ".edu.", ".gov."];

    let tld = rand::seq::SliceRandom::choose(&TLD[..], &mut rand::thread_rng())
        .unwrap();

    let mut nonce = [0_u8; 8];
    rand::Rng::fill(&mut rand::thread_rng(), &mut nonce);
    let mut name = String::new();
    for c in nonce {
        name.push_str(&format!("{c:02x}"));
    }
    name.push_str(tld);

    r.clear_cache();

    match r.lookup_ip(name).await {
        Ok(_) => OnlineEvent::Online,
        Err(err) => match err.kind() {
            ResolveErrorKind::NoRecordsFound { .. } => OnlineEvent::Online,
            _ => OnlineEvent::Offline,
        },
    }
}
