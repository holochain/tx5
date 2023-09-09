use std::borrow::Cow;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{atomic, Arc, Mutex};

/// The size of the text blocks
pub const BLOCK_I32: i32 = 32;
pub const BLOCK: usize = BLOCK_I32 as usize;

enum Coms<'lt> {
    /// "joined" peers send this notifying of updates they've made
    /// and requesting blocks if they have changes > last recvd versions
    ClientUpdate {
        want_blocks: Vec<(i32, i32, i32)>, // x, y, ver
        cur_pos: (i32, i32),               // x, y
        updates: Vec<(i32, i32, Node)>,    // x, y, node
    },

    /// "host" peers send this info updating the clients about the state
    ServerUpdate {
        cursors: Vec<(i32, i32, String)>, // x, y, nickname
        blocks: Vec<(
            i32, // x
            i32, // y
            i32, // ver
            Cow<'lt, [[Node; BLOCK]; BLOCK]>,
        )>,
    },
}

impl<'lt> Coms<'lt> {
    pub fn encode(&self, out: &mut String) {
        match self {
            Self::ClientUpdate {
                want_blocks,
                cur_pos,
                updates,
            } => {
                todo!()
            }
            Self::ServerUpdate { cursors, blocks } => {
                out.push_str(&format!("S\0{}\0", cursors.len()));
                for (x, y, nickname) in cursors {
                    out.push_str(&format!("{x}\0{y}\0{nickname}\0"));
                }
                out.push_str(&format!("{}\0", blocks.len()));
                for (x, y, ver, block) in blocks {
                    out.push_str(&format!("{x}\0{y}\0{ver}\0"));
                    for x in 0..BLOCK {
                        for y in 0..BLOCK {
                            block[x][y].encode(out);
                        }
                    }
                    out.push('\0');
                }
            }
        }
    }

    pub fn decode(&self, input: &str) -> Self {
        let mut input = input.split('\0');
        match input.next().unwrap() {
            "C" => {
                todo!()
            }
            "S" => {
                let cursor_cnt: usize = input.next().unwrap().parse().unwrap();
                let mut cursors = Vec::new();
                for _ in 0..cursor_cnt {
                    let x: i32 = input.next().unwrap().parse().unwrap();
                    let y: i32 = input.next().unwrap().parse().unwrap();
                    let nickname = input.next().unwrap().to_string();
                    cursors.push((x, y, nickname));
                }
                let block_cnt: usize = input.next().unwrap().parse().unwrap();
                let mut blocks = Vec::new();
                for _ in 0..block_cnt {
                    let x: i32 = input.next().unwrap().parse().unwrap();
                    let y: i32 = input.next().unwrap().parse().unwrap();
                    let ver: i32 = input.next().unwrap().parse().unwrap();
                    let mut out = [[Node::default(); BLOCK]; BLOCK];
                    let mut block = input.next().unwrap().chars();
                    for x in 0..BLOCK {
                        for y in 0..BLOCK {
                            let fg = block.next().unwrap();
                            let bg = block.next().unwrap();
                            let val = block.next().unwrap();
                            out[x][y] = Node {
                                fg: fg as u32 as u8,
                                bg: bg as u32 as u8,
                                val,
                            };
                        }
                    }
                    blocks.push((x, y, ver, Cow::Owned(out)));
                }
                Self::ServerUpdate { cursors, blocks }
            }
            _ => panic!(),
        }
    }
}

pub type BoxFut<R> = Pin<Box<dyn Future<Output = R> + 'static + Send>>;

pub trait StateHnd: 'static + Send + Sync {
    fn quit(&self);
    fn prompt(&self, state: &Arc<State>, p: String) -> BoxFut<String>;
    fn show_board(&self, state: &Arc<State>);
}

type DynStateHnd = Arc<dyn StateHnd + 'static + Send + Sync>;

pub const BLACK: char = '0';
pub const DARK_RED: char = '1';
pub const DARK_GREEN: char = '2';
pub const DARK_YELLOW: char = '3';
pub const DARK_BLUE: char = '4';
pub const DARK_MAGENTA: char = '5';
pub const DARK_CYAN: char = '6';
pub const DARK_GREY: char = '7';
pub const GREY: char = '8';
pub const RED: char = '9';
pub const GREEN: char = 'a';
pub const YELLOW: char = 'b';
pub const BLUE: char = 'c';
pub const MAGENTA: char = 'd';
pub const CYAN: char = 'e';
pub const WHITE: char = 'f';

#[derive(Debug, Clone, Copy)]
pub struct Node {
    pub fg: u8,
    pub bg: u8,
    pub val: char,
}

impl Default for Node {
    fn default() -> Self {
        Self {
            fg: WHITE as u32 as u8,
            bg: BLACK as u32 as u8,
            val: ' ',
        }
    }
}

impl Node {
    pub fn encode(&self, out: &mut String) {
        out.push(char::from_u32(self.fg as u32).unwrap());
        out.push(char::from_u32(self.bg as u32).unwrap());
        out.push(self.val);
    }
}

pub struct State {
    exit: atomic::AtomicBool,
    hnd: DynStateHnd,
    cli_url: String,
    nick: Mutex<String>,
    rem_host: Mutex<Option<String>>,
    blocks: Mutex<HashMap<(i32, i32), Arc<Mutex<[[Node; BLOCK]; BLOCK]>>>>,
    overlay: Mutex<HashMap<(i32, i32), (Node, i32)>>,
}

impl std::fmt::Debug for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("State")
            .field("cli_url", &self.cli_url)
            .field("nick", &*self.nick.lock().unwrap())
            .finish()
    }
}

impl State {
    pub async fn new<H: StateHnd>(hnd: H, cli_url: String) -> Arc<Self> {
        let hnd: DynStateHnd = Arc::new(hnd);

        let this = Arc::new(Self {
            exit: atomic::AtomicBool::new(false),
            hnd,
            cli_url,
            nick: Mutex::new("noname".to_string()),
            rem_host: Mutex::new(None),
            blocks: Mutex::new(HashMap::new()),
            overlay: Mutex::new(HashMap::new()),
        });

        this.clone().prompt_nick();

        this
    }

    pub fn fill_block(
        &self,
        offset_x: i32,
        offset_y: i32,
        block: &mut [[Node; BLOCK]; BLOCK],
    ) {
        if (offset_x / BLOCK_I32) * BLOCK_I32 != offset_x {
            panic!("invalid offset_x not divisible by block size");
        }
        if (offset_y / BLOCK_I32) * BLOCK_I32 != offset_y {
            panic!("invalid offset_y not divisible by BLOCK");
        }
        {
            let sblock = self
                .blocks
                .lock()
                .unwrap()
                .entry((offset_x, offset_y))
                .or_insert_with(|| {
                    Arc::new(Mutex::new([[Node::default(); BLOCK]; BLOCK]))
                })
                .clone();
            *block = *sblock.lock().unwrap();
        }
        let overlay_lock = self.overlay.lock().unwrap();
        for x in 0..BLOCK_I32 {
            for y in 0..BLOCK_I32 {
                if let Some((n, _)) =
                    overlay_lock.get(&(x + offset_x, y + offset_y))
                {
                    block[x as usize][y as usize] = *n;
                }
                if x == 0 || y == 0 {
                    block[x as usize][y as usize].val = '.';
                }
            }
        }
    }

    pub fn should_exit(&self) -> bool {
        self.exit.load(atomic::Ordering::Relaxed)
    }

    pub fn quit(&self) {
        self.exit.store(true, atomic::Ordering::Relaxed);
        self.hnd.quit();
    }

    // -- private -- //

    fn prompt_nick(self: Arc<Self>) {
        tokio::task::spawn(async move {
            let nick = self.hnd.prompt(&self, "nickname> ".to_string()).await;
            *self.nick.lock().unwrap() = nick;
            println!("nick: {}", self.nick.lock().unwrap());
            self.prompt_join_type();
        });
    }

    fn prompt_join_type(self: Arc<Self>) {
        tokio::task::spawn(async move {
            let prompt = format!(
                r#"-- Your Client URL if hosting --
{}
1) Host Private TextBoard
2) Join Private TextBoard
join type> "#,
                self.cli_url,
            );
            let jt = self.hnd.prompt(&self, prompt).await;

            match jt.as_str() {
                "1" => {
                    println!("join type: priv host");
                    self.start_board();
                }
                "2" => {
                    println!("join type: priv join");
                    self.prompt_join_priv_rem_url();
                }
                _ => {
                    eprintln!("invalid join type: {jt}");
                    self.prompt_join_type();
                }
            }
        });
    }

    fn prompt_join_priv_rem_url(self: Arc<Self>) {
        tokio::task::spawn(async move {
            let rem_url = self
                .hnd
                .prompt(&self, "remote client url> ".to_string())
                .await;
            println!("rem_url: {rem_url}");
            *self.rem_host.lock().unwrap() = Some(rem_url);
            self.start_board();
        });
    }

    fn start_board(self: Arc<Self>) {
        self.hnd.show_board(&self);
    }
}
