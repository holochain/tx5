use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

pub type BoxFut<R> = Pin<Box<dyn Future<Output = R> + 'static + Send>>;

pub trait StateHnd: 'static + Send + Sync {
    fn prompt(&self, p: String) -> BoxFut<String>;
    fn show_board(&self);
}

type DynStateHnd = Arc<dyn StateHnd + 'static + Send + Sync>;

#[derive(Clone, Copy)]
pub struct Node([u8; 8]);

impl Default for Node {
    fn default() -> Self {
        // the additional bytes are reserved for style
        // fg/bg color, etc
        Self([32, 0, 0, 0, 0, 0, 0, 0])
    }
}

impl std::fmt::Debug for Node {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.get_char().fmt(f)
    }
}

impl Node {
    /*
    #[inline(always)]
    pub fn set_char(&mut self, c: char) {
        self.0[0..4].copy_from_slice(&(c as u32).to_le_bytes());
    }

    #[inline(always)]
    pub fn with_char(mut self, c: char) -> Self {
        self.set_char(c);
        self
    }
    */

    #[inline(always)]
    pub fn get_char(&self) -> char {
        match char::from_u32(u32::from_le_bytes([
            self.0[0], self.0[1], self.0[2], self.0[3],
        ])) {
            Some(c) => c,
            None => ' ',
        }
    }
}

pub struct State {
    hnd: DynStateHnd,
    cli_url: String,
    nick: Mutex<String>,
    rem_host: Mutex<Option<String>>,
    exit: Arc<tokio::sync::Notify>,
    blocks: Mutex<HashMap<(i32, i32), Arc<Mutex<[[Node; 64]; 64]>>>>,
    overlay: Mutex<HashMap<(i32, i32), (Node, std::time::Instant)>>,
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
            hnd,
            cli_url,
            nick: Mutex::new("noname".to_string()),
            rem_host: Mutex::new(None),
            exit: Arc::new(tokio::sync::Notify::new()),
            blocks: Mutex::new(HashMap::new()),
            overlay: Mutex::new(HashMap::new()),
        });

        this.prompt_nick();

        this
    }

    pub fn fill_block(
        &self,
        offset_x: i32,
        offset_y: i32,
        block: &mut [[Node; 64]; 64],
    ) {
        if (offset_x / 64) * 64 != offset_x {
            panic!("invalid offset_x not divisible by 64");
        }
        if (offset_y / 64) * 64 != offset_y {
            panic!("invalid offset_y not divisible by 64");
        }
        {
            let sblock = self
                .blocks
                .lock()
                .unwrap()
                .entry((offset_x, offset_y))
                .or_insert_with(|| {
                    Arc::new(Mutex::new([[Node::default(); 64]; 64]))
                })
                .clone();
            *block = *sblock.lock().unwrap();
        }
        let overlay_lock = self.overlay.lock().unwrap();
        for x in 0..64 {
            for y in 0..64 {
                if let Some((n, _)) = overlay_lock.get(&(x + offset_x, y + offset_y)) {
                    block[x as usize][y as usize] = *n;
                }
            }
        }
    }

    pub async fn wait_exit(&self) {
        self.exit.notified().await;
    }

    fn prompt_nick(self: &Arc<Self>) {
        let this = self.clone();
        tokio::task::spawn(async move {
            let nick = this.hnd.prompt("nickname> ".to_string()).await;
            *this.nick.lock().unwrap() = nick;
            println!("nick: {}", this.nick.lock().unwrap());
            this.prompt_join_type();
        });
    }

    fn prompt_join_type(self: &Arc<Self>) {
        let this = self.clone();
        tokio::task::spawn(async move {
            let prompt = format!(
                r#"-- Your Client URL if hosting --
{}
1) Host Private TextBoard
2) Join Private TextBoard
join type> "#,
                this.cli_url,
            );
            let jt = this.hnd.prompt(prompt).await;

            match jt.as_str() {
                "1" => {
                    println!("join type: priv host");
                    this.hnd.show_board();
                }
                "2" => {
                    println!("join type: priv join");
                    this.prompt_join_priv_rem_url();
                }
                _ => {
                    eprintln!("invalid join type: {jt}");
                    this.prompt_join_type();
                }
            }
        });
    }

    fn prompt_join_priv_rem_url(self: &Arc<Self>) {
        let this = self.clone();
        tokio::task::spawn(async move {
            let rem_url =
                this.hnd.prompt("remote client url> ".to_string()).await;
            println!("rem_url: {rem_url}");
            *this.rem_host.lock().unwrap() = Some(rem_url);
            this.hnd.show_board();
        });
    }
}
