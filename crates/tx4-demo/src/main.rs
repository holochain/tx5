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
    /// Use a custom shoutout. Must be <= 16 utf8 bytes.
    #[clap(short, long, default_value = "Holochain rocks!")]
    pub shoutout: String,

    /// If specified, tracing logs will be written to the given file.
    /// You can use the environment variable `RUST_LOG` to control
    /// and filter the output. Defaults to INFO level.
    #[clap(short, long)]
    pub trace_file: Option<String>,

    /// Specify a display name. Must be <= 16 utf8 bytes.
    pub name: String,

    /// Signal server URL.
    pub sig_url: String,
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    if let Err(err) = main_err().await {
        eprintln!("{}", err);
        std::process::exit(1);
    }
}

enum TermEvt {
    Resize { x: u16, y: u16 },
    Backspace,
    Enter,
    Input(char),
    Output(String),
}

async fn main_err() -> Result<()> {
    let (t_send, mut t_recv) = tokio::sync::mpsc::unbounded_channel();
    let _ = t_send.send(TermEvt::Backspace);

    let Opt {
        shoutout,
        trace_file,
        name,
        sig_url,
    } = Opt::parse();

    let mut _app_guard = None;

    if let Some(trace_file) = trace_file {
        let trace_file = std::path::Path::new(&trace_file);
        let app = tracing_appender::rolling::never(
            trace_file
                .parent()
                .expect("failed to get dir from trace_file"),
            trace_file
                .file_name()
                .expect("failed to get filename from trace_file"),
        );
        let (app, g) =
            tracing_appender::non_blocking::NonBlockingBuilder::default()
                .lossy(false)
                .finish(app);

        _app_guard = Some(g);

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
    }

    if name.as_bytes().len() > 16 {
        return Err(Error::id("NameTooLong"));
    }

    if shoutout.as_bytes().len() > 16 {
        return Err(Error::id("ShoutoutTooLong"));
    }
    let sig_url = Tx4Url::new(sig_url)?;

    tracing::info!(%name, %shoutout, %sig_url);

    let shoutout = Arc::new(parking_lot::Mutex::new(shoutout));

    let (ep, mut evt) = tx4::Ep::new().await?;

    let addr = ep.listen(sig_url).await?;
    let this_id = addr.id().ok_or_else(|| Error::id("NoId"))?;

    tracing::info!(%addr);

    let state = State::new(this_id, t_send.clone());

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

                            // task this out so we don't backlog responses
                            let ep = ep.clone();
                            tokio::task::spawn(async move {
                                let two_msg =
                                    tx4::Buf::from_slice([2, 0]).unwrap();
                                if let Err(err) =
                                    ep.send(rem_cli_url, two_msg).await
                                {
                                    tracing::error!(?err);
                                }
                            });
                        }
                        tx4::EpEvt::Demo { rem_cli_url } => {
                            state.demo(rem_cli_url);
                        }
                    },
                }
            }
        });
    }

    // outgoing connections
    {
        let name = name.clone();
        let shoutout = shoutout.clone();
        let state = state.clone();
        let t_send = t_send.clone();
        tokio::task::spawn(async move {
            loop {
                match state.get_next_send() {
                    None => {
                        tokio::time::sleep(std::time::Duration::from_secs(1))
                            .await;
                    }
                    Some(url) => {
                        let so = shoutout.lock().clone();
                        let mut one_msg = Vec::with_capacity(
                            name.as_bytes().len() + so.as_bytes().len() + 3,
                        );
                        one_msg.push(1);
                        one_msg.push(0);
                        one_msg.extend_from_slice(name.as_bytes());
                        one_msg.push(0);
                        one_msg.extend_from_slice(so.as_bytes());
                        let one_msg = tx4::Buf::from_slice(&one_msg).unwrap();
                        if let Err(err) = ep.send(url.clone(), one_msg).await {
                            let _ = t_send.send(TermEvt::Output(format!(
                                "[ERR] ({:?}): {}",
                                url.id().unwrap(),
                                err,
                            )));
                            tracing::warn!(%url, ?err, "Send Error");
                        }
                    }
                }
            }
        });
    }

    use std::io::Write;
    let mut stdout = std::io::stdout();

    crossterm::terminal::enable_raw_mode()?;
    crossterm::execute!(stdout, crossterm::terminal::EnterAlternateScreen)?;
    crossterm::execute!(
        stdout,
        crossterm::terminal::Clear(crossterm::terminal::ClearType::All),
    )?;

    let (mut term_x, mut term_y) = crossterm::terminal::size()?;

    let res = tokio::select! {
        r = async move {
            tokio::task::spawn_blocking(move || {
                loop {
                    let evt = crossterm::event::read()?;
                    match evt {
                        crossterm::event::Event::Resize(x, y) => {
                            if t_send.send(TermEvt::Resize { x, y }).is_err() {
                                break;
                            }
                        }
                        crossterm::event::Event::Key(crossterm::event::KeyEvent {
                            code,
                            modifiers,
                            ..
                        }) => {
                            match code {
                                crossterm::event::KeyCode::Esc => {
                                    break;
                                }
                                crossterm::event::KeyCode::Enter => {
                                    if t_send.send(TermEvt::Enter).is_err() {
                                        break;
                                    }
                                }
                                crossterm::event::KeyCode::Backspace => {
                                    if t_send.send(TermEvt::Backspace).is_err() {
                                        break;
                                    }
                                }
                                crossterm::event::KeyCode::Char(c) => {
                                    use crossterm::event::KeyModifiers;

                                    if c == 'c' && modifiers.contains(KeyModifiers::CONTROL) {
                                        break;
                                    }

                                    if c == 'd' && modifiers.contains(KeyModifiers::CONTROL) {
                                        break;
                                    }

                                    if t_send.send(TermEvt::Input(c)).is_err() {
                                        break;
                                    }
                                }
                                _ => (),
                            }
                        }
                        _ => (),
                    }
                }
                Ok(())
            }).await?
        } => r,
        r = async move {
            let mut input = String::with_capacity(16);

            while let Some(evt) = t_recv.recv().await {
                match evt {
                    TermEvt::Resize { x, y } => {
                        term_x = x;
                        term_y = y;
                    }
                    TermEvt::Backspace => {
                        input.pop();
                    }
                    TermEvt::Enter => {
                        *shoutout.lock() = std::mem::take(&mut input);
                    }
                    TermEvt::Input(c) => {
                        input.push(c);
                        if input.as_bytes().len() > 16 {
                            input.pop();
                        }
                    }
                    TermEvt::Output(o) => {
                        crossterm::queue!(
                            stdout,
                            crossterm::cursor::MoveTo(0, term_y - 5),
                        )?;
                        crossterm::queue!(
                            stdout,
                            crossterm::terminal::Clear(crossterm::terminal::ClearType::CurrentLine),
                        )?;
                        write!(stdout, "{}", o)?;
                        crossterm::queue!(
                            stdout,
                            crossterm::terminal::ScrollUp(1),
                        )?;
                    }
                }

                crossterm::queue!(
                    stdout,
                    crossterm::cursor::MoveTo(0, term_y - 5),
                )?;
                for _ in 0..term_x {
                    write!(stdout, "-")?;
                }
                crossterm::queue!(
                    stdout,
                    crossterm::cursor::MoveTo(0, term_y - 4),
                )?;
                crossterm::queue!(
                    stdout,
                    crossterm::terminal::Clear(crossterm::terminal::ClearType::CurrentLine),
                )?;
                write!(stdout, "## Holochain Tx4 WebRTC Demo Cli ##")?;
                crossterm::queue!(
                    stdout,
                    crossterm::cursor::MoveTo(0, term_y - 3),
                )?;
                crossterm::queue!(
                    stdout,
                    crossterm::terminal::Clear(crossterm::terminal::ClearType::CurrentLine),
                )?;
                write!(
                    stdout,
                    "this: {} ({:?}): {}",
                    name,
                    this_id,
                    shoutout.lock(),
                )?;
                crossterm::queue!(
                    stdout,
                    crossterm::cursor::MoveTo(0, term_y - 2),
                )?;
                crossterm::queue!(
                    stdout,
                    crossterm::terminal::Clear(crossterm::terminal::ClearType::CurrentLine),
                )?;
                write!(
                    stdout,
                    "Active Connection Count: {}",
                    state.count(),
                )?;
                crossterm::queue!(
                    stdout,
                    crossterm::cursor::MoveTo(0, term_y - 1),
                )?;
                crossterm::queue!(
                    stdout,
                    crossterm::terminal::Clear(crossterm::terminal::ClearType::CurrentLine),
                )?;
                write!(stdout, "new shoutout> {}", input)?;

                stdout.flush()?;
            }
            Ok(())
        } => r,
    };

    crossterm::execute!(
        std::io::stdout(),
        crossterm::terminal::LeaveAlternateScreen,
    )?;
    crossterm::terminal::disable_raw_mode()?;

    res
}

#[derive(Clone)]
struct State(Arc<parking_lot::Mutex<StateInner>>);

impl State {
    pub fn new(
        this_id: Id,
        t_send: tokio::sync::mpsc::UnboundedSender<TermEvt>,
    ) -> Self {
        Self(Arc::new(parking_lot::Mutex::new(StateInner {
            this_id,
            map: HashMap::new(),
            queue: VecDeque::new(),
            t_send,
        })))
    }

    pub fn count(&self) -> usize {
        self.0.lock().map.len()
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

        let StateInner {
            map, queue, t_send, ..
        } = &mut *inner;

        match map.entry(url.clone()) {
            Occupied(mut e) => {
                let item = e.get_mut();
                if item.name != Some(name.clone())
                    || item.shoutout != Some(shoutout.clone())
                {
                    tracing::info!(%name, %shoutout, "Updated Info");
                    let _ = t_send.send(TermEvt::Output(format!(
                        "[UPD] {} ({:?}): {}",
                        name, id, shoutout,
                    )));
                    item.name = Some(name);
                    item.shoutout = Some(shoutout);
                }
            }
            Vacant(e) => {
                tracing::info!(%name, %shoutout, "New Peer");
                let _ = t_send.send(TermEvt::Output(format!(
                    "[NEW] {} ({:?}): {}",
                    name, id, shoutout,
                )));
                let mut item = Item::new(url.clone(), t_send.clone());
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

        let t_send = inner.t_send.clone();

        let _ = t_send.send(TermEvt::Output(format!("[NEW] ({:?})", id)));

        tracing::info!(%url, "demo");
        inner
            .map
            .insert(url.clone(), Item::new(url.clone(), t_send));
        inner.queue.push_back(url);
    }
}

struct Item {
    url: Tx4Url,
    last_send: std::time::Instant,
    last_recv: std::time::Instant,
    name: Option<String>,
    shoutout: Option<String>,
    t_send: tokio::sync::mpsc::UnboundedSender<TermEvt>,
}

impl Drop for Item {
    fn drop(&mut self) {
        tracing::info!(url = %self.url, name = ?self.name, shoutout = ?self.shoutout, "Dropping Peer");
        let name = self.name.as_deref().unwrap_or("");
        let shoutout = self.shoutout.as_deref().unwrap_or("");
        let _ = self.t_send.send(TermEvt::Output(format!(
            "[DRP] {} ({:?}): {}",
            name,
            self.url.id().unwrap(),
            shoutout,
        )));
    }
}

impl Item {
    pub fn new(
        url: Tx4Url,
        t_send: tokio::sync::mpsc::UnboundedSender<TermEvt>,
    ) -> Self {
        Self {
            url,
            last_send: std::time::Instant::now(),
            last_recv: std::time::Instant::now(),
            name: None,
            shoutout: None,
            t_send,
        }
    }
}

struct StateInner {
    this_id: Id,
    map: HashMap<Tx4Url, Item>,
    queue: VecDeque<Tx4Url>,
    t_send: tokio::sync::mpsc::UnboundedSender<TermEvt>,
}
