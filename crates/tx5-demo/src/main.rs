#![deny(warnings)]
#![deny(unsafe_code)]
#![allow(clippy::needless_range_loop)]
//! tx5-demo

use bytes::Buf;
use clap::Parser;
use std::collections::HashMap;
use tx5::{Ep, EpEvt, Result, Tx5Url};

#[derive(Debug, Parser)]
#[clap(name = "tx5-demo", version, about = "Holochain Tx5 WebRTC Demo Cli")]
struct Args {
    /// Tracing logs will be written to the given file.
    /// Any existing file will be deleted first.
    /// You can use the environment variable `RUST_LOG` to control
    /// and filter the output. Defaults to INFO level.
    pub trace_file: std::path::PathBuf,

    /// This node's address will be written to the given file.
    /// Any existing data will be truncated during write.
    pub addr_file: std::path::PathBuf,

    /// Signal server URL.
    pub sig_url: String,

    /// List of bootstrap peer client urls to connnect.
    pub peer_urls: Vec<String>,
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    if let Err(err) = main_err().await {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
enum Message {
    Hello { known_peers: Vec<String> },
    Big(String),
}

impl Message {
    pub fn encode(&self) -> Result<bytes::Bytes> {
        let b = serde_json::to_vec(self)?;
        let mut o = bytes::BytesMut::with_capacity(b.len());
        o.extend_from_slice(&b);
        Ok(o.freeze())
    }

    pub fn decode(data: bytes::Bytes) -> Result<Self> {
        Ok(serde_json::from_slice(&data)?)
    }

    pub fn hello(known_peers: &HashMap<Tx5Url, PeerInfo>) -> Self {
        Message::Hello {
            known_peers: known_peers.keys().map(|u| u.to_string()).collect(),
        }
    }

    pub fn big() -> Self {
        use rand::Rng;
        let mut big = vec![0; (1024 * 1024 * 15 * 3) / 4]; // 15 MiB but base64
        rand::thread_rng().fill(&mut big[..]);
        let big = base64::encode(&big);
        Message::Big(big)
    }
}

enum Lvl {
    Info,
    Error,
}

macro_rules! d {
    (info, $tag:literal) => {
        d!(@ (Lvl::Info) $tag "")
    };
    (info, $tag:literal, $($arg:tt)*) => {
        d!(@ (Lvl::Info) $tag format!($($arg)*))
    };
    (error, $tag:literal) => {
        d!(@ (Lvl::Error) $tag "")
    };
    (error, $tag:literal, $($arg:tt)*) => {
        d!(@ (Lvl::Error) $tag format!($($arg)*))
    };
    (@ ($lvl:path) $tag:literal $log:expr) => {{
        match $lvl {
            Lvl::Info => {
                tracing::info!("# {} # {} #", $tag, $log);
                println!(
                    "# tx5-demo # INFO # {} # {}:{} # {} #",
                    $tag,
                    file!(),
                    line!(),
                    $log,
                );
            }
            Lvl::Error => {
                tracing::error!("# {} # {} #", $tag, $log);
                println!(
                    "# tx5-demo # ERROR # {} # {}:{} # {} #",
                    $tag,
                    file!(),
                    line!(),
                    $log,
                );
            }
        }
    }};
}

struct PeerInfo {
    last_seen: std::time::Instant,
    last_sent: std::time::Instant,
}

impl std::fmt::Debug for PeerInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PeerInfo")
            .field("last_seen", &self.last_seen.elapsed().as_secs_f64())
            .field("last_sent", &self.last_sent.elapsed().as_secs_f64())
            .finish()
    }
}

impl PeerInfo {
    pub fn new() -> Self {
        Self {
            last_seen: std::time::Instant::now(),
            last_sent: std::time::Instant::now(),
        }
    }
}

struct Node {
    this_url: Tx5Url,
    known_peers: HashMap<Tx5Url, PeerInfo>,
}

impl std::fmt::Debug for Node {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut dbg = f.debug_struct("Node");
        dbg.field("this_id", &self.this_url.id().unwrap());
        for (url, i) in self.known_peers.iter() {
            dbg.field(&format!("{:?}", url.id().unwrap()), i);
        }
        dbg.finish()
    }
}

impl Node {
    pub fn new(this_url: Tx5Url, peer_urls: Vec<String>) -> Self {
        let known_peers = peer_urls
            .into_iter()
            .map(|u| (Tx5Url::new(u).unwrap(), PeerInfo::new()))
            .collect::<HashMap<_, _>>();
        Self {
            this_url,
            known_peers,
        }
    }

    pub fn add_known_peer(&mut self, url: Tx5Url) {
        if let std::collections::hash_map::Entry::Vacant(e) =
            self.known_peers.entry(url.clone())
        {
            e.insert(PeerInfo::new());
            d!(info, "DISCOVER", "{url}");
        }
    }

    pub fn send(&self, ep: &Ep, rem_url: &Tx5Url, data: bytes::Bytes) {
        let ep = ep.clone();
        let rem_url = rem_url.clone();
        tokio::task::spawn(async move {
            let len = data.remaining();
            let id = rem_url.id().unwrap();
            if let Err(err) = ep.send(rem_url, data).await {
                d!(
                    error,
                    "SEND_ERROR",
                    "len: {len}, dest: {id:?}, err: {err:?}"
                );
            } else {
                d!(info, "SEND_OK", "len: {len}, dest: {id:?}");
            }
        });
    }

    pub fn broadcast_hello(&self, ep: &Ep) -> Result<()> {
        let hello = Message::hello(&self.known_peers).encode()?;
        for url in self.known_peers.keys() {
            if url == &self.this_url {
                continue;
            }
            self.send(ep, url, hello.clone());
        }
        Ok(())
    }

    pub fn recv_hello(&mut self, url: Tx5Url) -> Result<()> {
        self.known_peers.get_mut(&url).unwrap().last_seen =
            std::time::Instant::now();
        d!(info, "RECV_HELLO", "{:?}", url.id().unwrap());
        Ok(())
    }

    pub fn five_sec(&mut self, ep: &Ep) -> Result<()> {
        {
            let this = self.known_peers.get_mut(&self.this_url).unwrap();
            this.last_seen = std::time::Instant::now();
            this.last_sent = std::time::Instant::now();
        }

        let mut v = self
            .known_peers
            .iter()
            .map(|(url, PeerInfo { last_sent, .. })| (url.clone(), last_sent))
            .collect::<Vec<_>>();
        v.sort_by(|a, b| a.1.cmp(b.1));
        let url = v.remove(0).0;
        if url == self.this_url {
            return Ok(());
        }
        self.known_peers.get_mut(&url).unwrap().last_sent =
            std::time::Instant::now();
        let hello = Message::hello(&self.known_peers).encode()?;
        self.send(ep, &url, hello);

        Ok(())
    }

    pub fn thirty_sec(&mut self, ep: &Ep) -> Result<()> {
        let mut v = Vec::new();

        for peer in self.known_peers.keys() {
            if peer != &self.this_url {
                v.push(peer);
            }
        }

        if v.is_empty() {
            return Ok(());
        }

        rand::seq::SliceRandom::shuffle(&mut v[..], &mut rand::thread_rng());
        let big = Message::big().encode()?;
        self.send(ep, v.get(0).unwrap(), big);

        Ok(())
    }
}

async fn main_err() -> Result<()> {
    let Args {
        trace_file,
        addr_file,
        sig_url,
        peer_urls,
    } = Args::parse();

    let mut addr_file = AddrFile::new(&addr_file).await?;

    let _ = std::fs::remove_file(&trace_file);

    let trace_file = std::path::Path::new(&trace_file);
    let app = tracing_appender::rolling::never(
        trace_file
            .parent()
            .expect("failed to get dir from trace_file"),
        trace_file
            .file_name()
            .expect("failed to get filename from trace_file"),
    );
    let (app, _app_guard) =
        tracing_appender::non_blocking::NonBlockingBuilder::default()
            .lossy(false)
            .finish(app);

    tracing_subscriber::FmtSubscriber::builder()
        .with_env_filter(
            tracing_subscriber::filter::EnvFilter::builder()
                .with_default_directive(
                    tracing_subscriber::filter::LevelFilter::INFO.into(),
                )
                .from_env_lossy(),
        )
        .with_file(true)
        .with_line_number(true)
        .with_writer(app)
        .init();

    let sig_url = Tx5Url::new(sig_url)?;

    let (ep, mut evt) = tx5::Ep::new().await?;
    let this_addr = ep.listen(sig_url.clone()).await?;

    let mut node = Node::new(this_addr.clone(), peer_urls);

    node.add_known_peer(this_addr.clone());

    node.broadcast_hello(&ep)?;

    addr_file.write(&this_addr).await?;
    drop(addr_file);

    d!(info, "STARTED", "{this_addr} {node:?}");

    enum Cmd {
        FiveSec,
        ThirtySec,
        EpEvt(Result<EpEvt>),
    }

    let (cmd_s, mut cmd_r) = tokio::sync::mpsc::unbounded_channel();

    {
        let cmd_s = cmd_s.clone();
        tokio::task::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                if cmd_s.send(Cmd::FiveSec).is_err() {
                    return;
                }
            }
        });
    }

    {
        let cmd_s = cmd_s.clone();
        tokio::task::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                if cmd_s.send(Cmd::ThirtySec).is_err() {
                    return;
                }
            }
        });
    }

    {
        let cmd_s = cmd_s.clone();
        tokio::task::spawn(async move {
            while let Some(evt) = evt.recv().await {
                if cmd_s.send(Cmd::EpEvt(evt)).is_err() {
                    return;
                }
            }
        });
    }

    while let Some(cmd) = cmd_r.recv().await {
        match cmd {
            Cmd::FiveSec => {
                node.five_sec(&ep)?;
                d!(info, "FIVE_SEC", "{node:?}");
                tracing::info!(
                    "{}",
                    serde_json::to_string(&ep.get_stats().await.unwrap())
                        .unwrap()
                );
            }
            Cmd::ThirtySec => node.thirty_sec(&ep)?,
            Cmd::EpEvt(Err(err)) => panic!("{err:?}"),
            Cmd::EpEvt(Ok(EpEvt::Connected { rem_cli_url })) => {
                node.add_known_peer(rem_cli_url);
            }
            Cmd::EpEvt(Ok(EpEvt::Disconnected { rem_cli_url })) => {
                d!(info, "DISCONNECTED", "{:?}", rem_cli_url.id().unwrap());
            }
            Cmd::EpEvt(Ok(EpEvt::Data {
                rem_cli_url,
                mut data,
                ..
            })) => {
                node.add_known_peer(rem_cli_url.clone());
                match Message::decode(data.copy_to_bytes(data.remaining())) {
                    Err(err) => panic!("{err:?}"),
                    Ok(Message::Hello { known_peers: kp }) => {
                        for peer in kp {
                            node.add_known_peer(Tx5Url::new(peer).unwrap());
                        }
                        node.recv_hello(rem_cli_url)?;
                    }
                    Ok(Message::Big(d)) => {
                        d!(
                            info,
                            "RECV_BIG",
                            "len:{} {:?}",
                            d.as_bytes().len(),
                            rem_cli_url.id().unwrap()
                        );
                    }
                }
            }
            Cmd::EpEvt(_) => (),
        }
    }

    Ok(())
}

struct AddrFile(tokio::fs::File);

impl AddrFile {
    pub async fn new(path: &std::path::Path) -> Result<Self> {
        Ok(Self(tokio::fs::File::create(path).await?))
    }

    pub async fn write(&mut self, this_addr: &Tx5Url) -> Result<()> {
        use tokio::io::AsyncSeekExt;
        use tokio::io::AsyncWriteExt;

        self.0.seek(std::io::SeekFrom::Start(0)).await?;
        self.0.set_len(0).await?;

        self.0
            .write_all(<Tx5Url as AsRef<str>>::as_ref(this_addr).as_bytes())
            .await?;
        self.0.write_all(b"\n").await?;
        self.0.sync_all().await?;
        Ok(())
    }
}
