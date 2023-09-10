use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::{atomic, Arc, Mutex};

pub fn debug_write(s: String) {
    static F: std::sync::OnceLock<Arc<tokio::sync::Mutex<tokio::fs::File>>> =
        std::sync::OnceLock::new();

    tokio::task::spawn(async move {
        use tokio::io::AsyncWriteExt;

        let f = F.get_or_init(|| {
            let f = std::fs::File::create("./textboard-debug.txt").unwrap();
            let f = tokio::fs::File::from_std(f);
            Arc::new(tokio::sync::Mutex::new(f))
        });
        let mut lock = f.lock().await;

        lock.write_all(s.as_bytes()).await.unwrap();
        lock.write_all(b"\n").await.unwrap();
        lock.flush().await.unwrap();
    });
}

/// The size of the text blocks
pub const BLOCK_I32: i32 = 32;
pub const BLOCK: usize = BLOCK_I32 as usize;

#[derive(Debug)]
enum Coms<'lt> {
    /// "joined" peers send this notifying of updates they've made
    /// and requesting blocks if they have changes > last recvd versions
    ClientUpdate {
        client_ver: i32, // this is returned in ServerUpdate
        want_blocks: Vec<(i32, i32, i32)>,      // x, y, block ver
        cursor: (i32, i32, String),        // x, y, nick
        updates: Vec<(i32, i32, Node)>,    // x, y, node
    },

    /// "host" peers send this info updating the clients about the state
    ServerUpdate {
        client_ver: i32, // this is the version sent in the ClientUpdate
        cursors: Vec<(i32, i32, String)>, // x, y, nickname
        blocks: Vec<(
            i32, // x
            i32, // y
            i32, // block ver
            Cow<'lt, [[Node; BLOCK]; BLOCK]>,
        )>,
    },
}

impl<'lt> Coms<'lt> {
    pub fn encode(&self, out: &mut String) {
        match self {
            Self::ClientUpdate {
                client_ver,
                want_blocks,
                cursor,
                updates,
            } => {
                out.push_str(&format!("C\0{client_ver}\0{}\0", want_blocks.len()));
                for (x, y, ver) in want_blocks {
                    out.push_str(&format!("{x}\0{y}\0{ver}\0"));
                }
                out.push_str(&format!(
                    "{}\0{}\0{}\0{}\0",
                    cursor.0,
                    cursor.1,
                    cursor.2,
                    updates.len(),
                ));
                for (x, y, node) in updates {
                    out.push_str(&format!("{x}\0{y}\0"));
                    node.encode(out);
                    out.push('\0');
                }
            }
            Self::ServerUpdate { client_ver, cursors, blocks } => {
                out.push_str(&format!("S\0{client_ver}\0{}\0", cursors.len()));
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
                let client_ver: i32 = input.next().unwrap().parse().unwrap();
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
                Self::ServerUpdate { client_ver, cursors, blocks }
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
    dirty: atomic::AtomicBool,
    hnd: DynStateHnd,
    cli_url: String,
    nick: Mutex<String>,
    rem_host: Mutex<Option<String>>,
    blocks: Mutex<HashMap<(i32, i32), Arc<Mutex<([[Node; BLOCK]; BLOCK], i32)>>>>,
    overlay: Mutex<HashMap<(i32, i32), (Node, i32)>>,
    this_cur: Mutex<(i32, i32)>,
    view_tracking: Mutex<HashSet<(i32, i32)>>,
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
            dirty: atomic::AtomicBool::new(true),
            hnd,
            cli_url,
            nick: Mutex::new("noname".to_string()),
            rem_host: Mutex::new(None),
            blocks: Mutex::new(HashMap::new()),
            overlay: Mutex::new(HashMap::new()),
            this_cur: Mutex::new((0, 0)),
            view_tracking: Mutex::new(HashSet::new()),
        });

        this.clone().prompt_nick();

        this.move_cur(0, -6);
        this.enter();
        for s in [
            "Welcome to textboard.",
            "[ESC] to exit.",
            "[ENTER] does columns.",
            "[SHIFT]+[ARROW] moves faster.",
            "Have fun!",
        ] {
            for c in s.chars() {
                this.write(Node {
                    fg: CYAN as u32 as u8,
                    bg: BLACK as u32 as u8,
                    val: c,
                });
            }
            this.enter();
        }

        this
    }

    pub fn set_view_tracking(&self, mut tracking: HashSet<(i32, i32)>) {
        std::mem::swap(&mut *self.view_tracking.lock().unwrap(), &mut tracking);
    }

    pub fn fill_block(
        &self,
        offset_x: i32,
        offset_y: i32,
        block: &mut [[Node; BLOCK]; BLOCK],
    ) {
        *block = self.get_block(offset_x, offset_y).lock().unwrap().0;
        let overlay_lock = self.overlay.lock().unwrap();
        for x in 0..BLOCK_I32 {
            for y in 0..BLOCK_I32 {
                if let Some((n, _)) =
                    overlay_lock.get(&(x + offset_x, y + offset_y))
                {
                    block[x as usize][y as usize] = *n;
                }
            }
        }
    }

    pub fn should_exit(&self) -> bool {
        self.exit.load(atomic::Ordering::Relaxed)
    }

    pub fn should_draw(&self) -> Option<Vec<(i32, i32, String)>> {
        if self.dirty.swap(false, atomic::Ordering::Relaxed) {
            let (x, y) = *self.this_cur.lock().unwrap();
            let nick = self.nick.lock().unwrap().clone();
            Some(vec![(x, y, nick)])
        } else {
            None
        }
    }

    pub fn quit(&self) {
        self.exit.store(true, atomic::Ordering::Relaxed);
        self.hnd.quit();
    }

    pub fn write(&self, node: Node) {
        let (x, y) = {
            let mut c_lock = self.this_cur.lock().unwrap();
            let out = *c_lock;
            (*c_lock).0 += 1;
            out
        };
        self.overlay.lock().unwrap().insert((x, y), (node, 0));
        self.set_dirty();
    }

    pub fn backspace(&self) {
        let (x, y) = {
            let mut c_lock = self.this_cur.lock().unwrap();
            (*c_lock).0 -= 1;
            *c_lock
        };
        self.overlay.lock().unwrap().insert((x, y), (Node {
            fg: WHITE as u32 as u8,
            bg: BLACK as u32 as u8,
            val: ' ',
        }, 0));
        self.set_dirty();
    }

    pub fn move_cur(&self, dx: i32, dy: i32) {
        {
            let mut c_lock = self.this_cur.lock().unwrap();
            (*c_lock).0 += dx;
            (*c_lock).1 += dy;
        }
        self.set_dirty();
    }

    pub fn enter(&self) {
        let (x, y) = {
            let mut c_lock = self.this_cur.lock().unwrap();
            let x = (*c_lock).0;
            let mut nx = (x / 32) * 32;
            if nx > x {
                nx -= 32;
            }
            (*c_lock).0 = nx;
            (*c_lock).1 += 1;
            *c_lock
        };
        {
            let mut overlay = self.overlay.lock().unwrap();
            overlay.insert((x - 2, y), (Node {
                fg: DARK_GREY as u32 as u8,
                bg: BLACK as u32 as u8,
                val: '│',
            }, 0));
            overlay.insert((x + 30, y), (Node {
                fg: DARK_GREY as u32 as u8,
                bg: BLACK as u32 as u8,
                val: '│',
            }, 0));
        }
        self.set_dirty();
    }

    // -- private -- //

    fn write_block(&self, x: i32, y: i32, node: Node) {
        let mut cx = (x / BLOCK_I32) * BLOCK_I32;
        if cx > x {
            cx -= BLOCK_I32;
        }
        let mut cy = (y / BLOCK_I32) * BLOCK_I32;
        if cy > y {
            cy -= BLOCK_I32;
        }
        let block = self.get_block(cx, cy);
        let ix = x - cx;
        let iy = y - cy;
        let mut lock = block.lock().unwrap();
        lock.0[ix as usize][iy as usize] = node;
        lock.1 += 1;
    }

    fn set_dirty(&self) {
        self.dirty.store(true, atomic::Ordering::Relaxed);
    }

    fn get_block(
        &self,
        offset_x: i32,
        offset_y: i32,
    ) -> Arc<Mutex<([[Node; BLOCK]; BLOCK], i32)>> {
        if (offset_x / BLOCK_I32) * BLOCK_I32 != offset_x {
            panic!("invalid offset_x not divisible by block size");
        }
        if (offset_y / BLOCK_I32) * BLOCK_I32 != offset_y {
            panic!("invalid offset_y not divisible by BLOCK");
        }
        self
            .blocks
            .lock()
            .unwrap()
            .entry((offset_x, offset_y))
            .or_insert_with(|| {
                Arc::new(Mutex::new(([[Node::default(); BLOCK]; BLOCK], 1)))
            })
            .clone()
    }

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

        if !self.rem_host.lock().unwrap().is_none() {
            tokio::task::spawn(self.manage_overlay_as_host());
        } else {
            tokio::task::spawn(self.manage_overlay_as_client());
        }
    }

    async fn manage_overlay_as_host(self: Arc<Self>) {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;

            // todo - first integrate changes from clients
            // the host gets priority so host changes come after

            // now, integrate host changes so they overwrite
            let overlay: Vec<((i32, i32), (Node, i32))> =
                self.overlay.lock().unwrap().drain().collect();

            for ((x, y), (node, _)) in overlay {
                self.write_block(x, y, node);
            }
        }
    }

    async fn manage_overlay_as_client(self: Arc<Self>) {
        let mut next_client_ver = 1;

        loop {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;

            let client_ver = next_client_ver;
            next_client_ver += 1;

            let view_tracking: Vec<(i32, i32)> = self.view_tracking.lock().unwrap().iter().cloned().collect();

            let mut want_blocks: Vec<(i32, i32, i32)> = Vec::new();

            {
                for (x, y) in view_tracking {
                    let block = self.get_block(x, y);
                    let block = block.lock().unwrap();
                    want_blocks.push((x, y, block.1));
                }
            }

            let (x, y) = *self.this_cur.lock().unwrap();
            let nick = self.nick.lock().unwrap().clone();
            let cursor: (i32, i32, String) = (x, y, nick);

            let mut updates: Vec<(i32, i32, Node)> = Vec::new();

            {
                let mut lock = self.overlay.lock().unwrap();
                for ((x, y), (node, ver)) in lock.iter_mut() {
                    if *ver == 0 {
                        *ver = client_ver;
                        updates.push((*x, *y, *node));
                    }
                }
            }

            let com = Coms::ClientUpdate {
                client_ver,
                want_blocks,
                cursor,
                updates,
            };

            let mut enc = String::new();
            com.encode(&mut enc);

            debug_write(format!("{com:?}\n{enc}"));
        }
    }
}

