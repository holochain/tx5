#![deny(warnings)]
#![deny(unsafe_code)]
#![allow(clippy::needless_range_loop)]

use clap::Parser;
use std::collections::hash_map::Entry::*;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use tx4::{Error, Id, Result, Tx4Url};

#[derive(Debug, Parser)]
#[clap(name = "tx4-demo", version, about = "Holochain Tx4 WebRTC Demo Cli")]
pub struct Opt {
    /// Use a custom username. Must be < 128 utf8 bytes.
    #[clap(short, long, default_value = "Holochain<3")]
    pub name: String,

    /// Use a custom shoutout. Must be < 128 utf8 bytes.
    #[clap(short, long, default_value = "Holochain rocks!")]
    pub shoutout: String,

    /// Signal server URL.
    pub sig_url: String,
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let subscriber = tracing_subscriber::FmtSubscriber::builder()
        .with_env_filter(
            tracing_subscriber::filter::EnvFilter::builder()
                .with_default_directive(
                    tracing_subscriber::filter::LevelFilter::INFO.into(),
                )
                .from_env_lossy(),
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

async fn main_err() -> Result<()> {
    let Opt {
        name,
        shoutout,
        sig_url,
    } = Opt::parse();

    if name.as_bytes().len() > 128 {
        return Err(Error::id("NameTooLong"));
    }

    if shoutout.as_bytes().len() > 128 {
        return Err(Error::id("ShoutoutTooLong"));
    }

    let sig_url = Tx4Url::new(sig_url)?;

    tracing::info!(%name, %shoutout, %sig_url);

    let (ep, mut evt) = tx4::Ep::new().await?;

    let addr = ep.listen(sig_url).await?;
    let this_id = addr.id().ok_or_else(|| Error::id("NoId"))?;

    tracing::info!(%addr);

    let mut one_msg = Vec::with_capacity(
        name.as_bytes().len() + shoutout.as_bytes().len() + 3,
    );
    one_msg.push(1);
    one_msg.push(0);
    one_msg.extend_from_slice(name.as_bytes());
    one_msg.push(0);
    one_msg.extend_from_slice(shoutout.as_bytes());

    let mut two_msg = one_msg.clone();
    two_msg[0] = 2;

    let mut one_msg = tx4::Buf::from_slice(&one_msg)?;
    let mut two_msg = tx4::Buf::from_slice(&two_msg)?;

    let state = State::new(this_id);

    // print summary every 15 secs
    {
        let state = state.clone();
        tokio::task::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(15)).await;
                tracing::info!("{}", state.summary());
            }
        });
    }

    // demo broadcast every 5 secs
    {
        let ep = ep.clone();
        tokio::task::spawn(async move {
            loop {
                if ep.demo().is_err() {
                    break;
                }

                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
        });
    }

    // event receiver
    {
        let ep = ep.clone();
        let state = state.clone();
        tokio::task::spawn(async move {
            while let Some(evt) = evt.recv().await {
                match evt {
                    Err(err) => tracing::error!(?err),
                    Ok(evt) => match evt {
                        tx4::EpEvt::Data {
                            rem_cli_url,
                            mut data,
                            ..
                        } => {
                            let data = data.to_vec().unwrap();
                            if data.len() < 3 {
                                continue;
                            }
                            if data[0] == 2 {
                                state.recv(rem_cli_url);
                                continue;
                            }
                            if data[0] != 1 {
                                continue;
                            }

                            if data[1] != 0 {
                                continue;
                            }

                            let mut idx = 2;
                            for i in 2..data.len() {
                                if data[i] == 0 {
                                    idx = i;
                                    break;
                                }
                            }

                            let name = String::from_utf8_lossy(&data[2..idx])
                                .to_string();
                            let shoutout =
                                String::from_utf8_lossy(&data[idx + 1..])
                                    .to_string();

                            state.handle(rem_cli_url.clone(), name, shoutout);

                            if let Err(err) = ep
                                .send(rem_cli_url, two_msg.try_clone().unwrap())
                                .await
                            {
                                tracing::error!(?err);
                            }
                        }
                        tx4::EpEvt::Demo { rem_cli_url } => {
                            state.demo(rem_cli_url);
                        }
                    },
                }
            }
        });
    }

    loop {
        match state.get_next_send() {
            None => {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
            Some(url) => {
                if let Err(err) =
                    ep.send(url.clone(), one_msg.try_clone().unwrap()).await
                {
                    tracing::warn!(%url, ?err, "Send Error");
                }
            }
        }
    }
}

#[derive(Clone)]
struct State(Arc<parking_lot::Mutex<StateInner>>);

impl State {
    pub fn new(this_id: Id) -> Self {
        Self(Arc::new(parking_lot::Mutex::new(StateInner {
            this_id,
            map: HashMap::new(),
            queue: VecDeque::new(),
        })))
    }

    pub fn get_next_send(&self) -> Option<Tx4Url> {
        let mut inner = self.0.lock();

        inner.map.retain(|_, item| {
            item.last_recv.elapsed() <= std::time::Duration::from_secs(20)
        });

        let count = inner.queue.len();
        for _ in 0..count {
            let url = match inner.queue.pop_front() {
                None => return None,
                Some(url) => url,
            };

            let StateInner { map, queue, .. } = &mut *inner;

            if let Some(item) = map.get_mut(&url) {
                queue.push_back(url.clone());
                if item.last_send.elapsed() > std::time::Duration::from_secs(5)
                {
                    item.last_send = std::time::Instant::now();
                    return Some(url);
                }
            }
        }

        None
    }

    pub fn handle(&self, url: Tx4Url, name: String, shoutout: String) {
        let id = match url.id() {
            None => return,
            Some(id) => id,
        };

        let mut inner = self.0.lock();

        if id == inner.this_id {
            return;
        }

        let StateInner { map, queue, .. } = &mut *inner;

        match map.entry(url.clone()) {
            Occupied(mut e) => {
                let item = e.get_mut();
                if item.name != Some(name.clone())
                    || item.shoutout != Some(shoutout.clone())
                {
                    tracing::info!(%name, %shoutout, "Updated Info");
                    item.name = Some(name);
                    item.shoutout = Some(shoutout);
                }
            }
            Vacant(e) => {
                tracing::info!(%name, %shoutout, "New Peer");
                let mut item = Item::new(url.clone());
                item.name = Some(name);
                item.shoutout = Some(shoutout);
                e.insert(item);
                queue.push_back(url);
            }
        }
    }

    pub fn recv(&self, url: Tx4Url) {
        let id = match url.id() {
            None => return,
            Some(id) => id,
        };

        let mut inner = self.0.lock();

        if id == inner.this_id {
            return;
        }

        if let Some(item) = inner.map.get_mut(&url) {
            item.last_recv = std::time::Instant::now();
        }
    }

    pub fn demo(&self, url: Tx4Url) {
        let id = match url.id() {
            None => return,
            Some(id) => id,
        };

        let mut inner = self.0.lock();

        if id == inner.this_id {
            return;
        }

        if inner.map.contains_key(&url) {
            return;
        }

        tracing::info!(%url, "demo");
        inner.map.insert(url.clone(), Item::new(url.clone()));
        inner.queue.push_back(url);
    }

    pub fn summary(&self) -> String {
        let inner = self.0.lock();
        let mut out = format!("{} Active Peers:\n", inner.map.len());
        for (_, item) in inner.map.iter() {
            let name = item.name.as_deref().unwrap_or("");
            let shoutout = item.shoutout.as_deref().unwrap_or("");
            out.push_str(&format!(
                " - {}({:?}) {}\n",
                name, shoutout, item.url,
            ));
        }
        out
    }
}

struct Item {
    url: Tx4Url,
    last_send: std::time::Instant,
    last_recv: std::time::Instant,
    name: Option<String>,
    shoutout: Option<String>,
}

impl Drop for Item {
    fn drop(&mut self) {
        tracing::info!(url = %self.url, name = ?self.name, shoutout = ?self.shoutout, "Dropping Peer");
    }
}

impl Item {
    pub fn new(url: Tx4Url) -> Self {
        Self {
            url,
            last_send: std::time::Instant::now(),
            last_recv: std::time::Instant::now(),
            name: None,
            shoutout: None,
        }
    }
}

struct StateInner {
    this_id: Id,
    map: HashMap<Tx4Url, Item>,
    queue: VecDeque<Tx4Url>,
}
