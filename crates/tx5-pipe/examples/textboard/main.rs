use std::sync::{Arc, Mutex, atomic};
use std::collections::HashSet;

mod state;
use bytes::*;
use state::*;

static FG: atomic::AtomicU8 = atomic::AtomicU8::new(state::WHITE as u32 as u8);
static BG: atomic::AtomicU8 = atomic::AtomicU8::new(state::BLACK as u32 as u8);

fn get_fg() -> u8 {
    FG.load(atomic::Ordering::Relaxed)
}

fn get_bg() -> u8 {
    BG.load(atomic::Ordering::Relaxed)
}

fn set_fg(fg: u8) {
    FG.store(fg, atomic::Ordering::Relaxed);
}

fn set_bg(bg: u8) {
    BG.store(bg, atomic::Ordering::Relaxed);
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    if !crossterm::tty::IsTty::is_tty(&std::io::stdout()) {
        eprintln!("textboard is a command-line tty application, no piping");
        std::process::exit(127);
    }

    struct PHnd;

    impl tx5_pipe_control::Tx5PipeControlHandler for PHnd {
        fn recv(&self, _: String, _: Box<(dyn Buf + Send + 'static)>) {
            todo!()
        }
    }

    let pipe = tx5_pipe::Tx5Pipe::spawn_in_process(PHnd).await.unwrap();

    /*
    let cli_url = pipe
        .sig_reg("wss://signal.holo.host".to_string())
        .await
        .unwrap();
    */
    let cli_url = "bla".to_string();

    struct SHnd {
        pipe: tx5_pipe_control::Tx5PipeControl,
        board_task: Arc<
            Mutex<
                Option<
                    tokio::task::JoinHandle<
                        std::result::Result<
                            std::io::Result<
                                tokio::task::JoinHandle<
                                    std::result::Result<
                                        std::io::Result<()>,
                                        tokio::task::JoinError,
                                    >,
                                >,
                            >,
                            tokio::task::JoinError,
                        >,
                    >,
                >,
            >,
        >,
    }

    let board_task = Arc::new(Mutex::new(None));

    impl StateHnd for SHnd {
        fn quit(&self) {
            self.pipe.quit();
        }

        fn prompt(&self, _: &Arc<State>, p: String) -> BoxFut<String> {
            use std::io::Write;
            Box::pin(async move {
                tokio::task::spawn_blocking(move || {
                    {
                        let mut stdout = std::io::stdout().lock();
                        write!(stdout, "{p}").unwrap();
                        stdout.flush().unwrap();
                    }
                    let mut result = String::new();
                    std::io::stdin().read_line(&mut result).unwrap();
                    result.trim().to_string()
                })
                .await
                .unwrap()
            })
        }

        fn show_board(&self, state: &Arc<State>) {
            let state = state.clone();
            *self.board_task.lock().unwrap() = Some(tokio::task::spawn(
                tokio::task::spawn_blocking(move || run_board(state.clone())),
            ));
        }
    }

    let state = State::new(
        SHnd {
            pipe,
            board_task: board_task.clone(),
        },
        cli_url,
    )
    .await;

    while !state.should_exit() {
        tokio::time::sleep(std::time::Duration::from_millis(17)).await;
    }

    if let Some(board_task) = board_task.lock().unwrap().take() {
        board_task
            .await
            .unwrap()
            .unwrap()
            .unwrap()
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    };
}

fn run_event(state: Arc<State>) -> std::io::Result<()> {
    use crossterm::event::*;

    loop {
        if poll(std::time::Duration::from_millis(17))? {
            match read()? {
                Event::Key(event) => {
                    if event.code == KeyCode::Esc
                        || (event.code == KeyCode::Char('c')
                            && event.modifiers.contains(KeyModifiers::CONTROL))
                    {
                        state.quit();
                    } else if let KeyCode::F(1) = event.code {
                        set_fg(inc_color(get_fg()));
                        state.move_cur(0, 0);
                    } else if let KeyCode::F(2) = event.code {
                        set_bg(inc_color(get_bg()));
                        state.move_cur(0, 0);
                    } else if let KeyCode::F(3) = event.code {
                        set_fg(state::WHITE as u32 as u8);
                        set_bg(state::BLACK as u32 as u8);
                        state.move_cur(0, 0);
                    } else if event.code == KeyCode::Backspace {
                        state.backspace();
                    } else if event.code == KeyCode::Enter {
                        state.enter();
                    } else if event.code == KeyCode::Up {
                        let dy = if event.modifiers.contains(KeyModifiers::SHIFT) {
                            -8
                        } else {
                            -1
                        };
                        state.move_cur(0, dy);
                    } else if event.code == KeyCode::Down {
                        let dy = if event.modifiers.contains(KeyModifiers::SHIFT) {
                            8
                        } else {
                            1
                        };
                        state.move_cur(0, dy);
                    } else if event.code == KeyCode::Left {
                        let dx = if event.modifiers.contains(KeyModifiers::SHIFT) {
                            -16
                        } else {
                            -1
                        };
                        state.move_cur(dx, 0);
                    } else if event.code == KeyCode::Right {
                        let dx = if event.modifiers.contains(KeyModifiers::SHIFT) {
                            16
                        } else {
                            1
                        };
                        state.move_cur(dx, 0);
                    } else if let KeyCode::Char(c) = event.code {
                        state.write(state::Node {
                            fg: get_fg(),
                            bg: get_bg(),
                            val: c,
                        });
                    }
                }
                _ => (),
            }
        }

        if state.should_exit() {
            break;
        }
    }

    Ok(())
}

fn to_color(c: u8, is_fg: bool) -> crossterm::style::Color {
    use crossterm::style::Color;
    match char::from_u32(c as u32).unwrap() {
        state::BLACK  => Color::Black,
        state::DARK_RED  => Color::DarkRed,
        state::DARK_GREEN => Color::DarkGreen,
        state::DARK_YELLOW  => Color::DarkYellow,
        state::DARK_BLUE  => Color::DarkBlue,
        state::DARK_MAGENTA  => Color::DarkMagenta,
        state::DARK_CYAN  => Color::DarkCyan,
        state::DARK_GREY  => Color::DarkGrey,
        state::GREY  => Color::Grey,
        state::RED  => Color::Red,
        state::GREEN  => Color::Green,
        state::YELLOW  => Color::Yellow,
        state::BLUE  => Color::Blue,
        state::MAGENTA  => Color::Magenta,
        state::CYAN  => Color::Cyan,
        state::WHITE  => Color::White,
        _ => if is_fg { Color::White } else { Color::Black },
    }
}

fn inc_color(mut c: u8) -> u8 {
    c += 1;
    if c == 58 {
        c = 97;
    } else if c == 103 {
        c = 48;
    }
    c
}

fn run_board(
    state: Arc<State>,
) -> std::io::Result<
    tokio::task::JoinHandle<
        std::result::Result<std::io::Result<()>, tokio::task::JoinError>,
    >,
> {
    use std::io::Write;

    struct OnDrop;

    impl Drop for OnDrop {
        fn drop(&mut self) {
            let mut stdout = std::io::stdout().lock();

            let _ = crossterm::execute!(
                stdout,
                crossterm::cursor::Show,
                crossterm::terminal::EnableLineWrap,
                crossterm::terminal::LeaveAlternateScreen,
            );

            let _ = crossterm::terminal::disable_raw_mode();
        }
    }

    let _on_drop = OnDrop;

    {
        let mut stdout = std::io::stdout().lock();

        crossterm::terminal::enable_raw_mode()?;

        crossterm::execute!(
            stdout,
            crossterm::terminal::EnterAlternateScreen,
            crossterm::terminal::DisableLineWrap,
            crossterm::cursor::Hide,
        )?;
    }

    let mut offset_x: i32;
    let mut offset_y: i32;

    let event_state = state.clone();
    let event_task =
        tokio::task::spawn(tokio::task::spawn_blocking(move || {
            run_event(event_state)
        }));

    let mut block = [[state::Node::default(); state::BLOCK]; state::BLOCK];

    loop {
        if let Some(cursors) = state.should_draw() {
            let (width, height) = crossterm::terminal::size()?;
            let width = width as i32;
            let height = height as i32;

            let this_cur_x = cursors[0].0;
            let this_cur_y = cursors[0].1;

            offset_x = this_cur_x - (width / 2);
            offset_y = this_cur_y - (height / 2);

            let mut stdout = std::io::stdout().lock();

            crossterm::queue!(
                stdout,
                crossterm::terminal::BeginSynchronizedUpdate,
                crossterm::terminal::Clear(crossterm::terminal::ClearType::Purge),
                crossterm::style::ResetColor,
                crossterm::cursor::MoveTo(1, 1),
            )?;

            let mut cx = (offset_x / state::BLOCK_I32) * state::BLOCK_I32;
            if cx > offset_x {
                cx -= state::BLOCK_I32;
            }

            let mut cy = (offset_y / state::BLOCK_I32) * state::BLOCK_I32;
            if cy > offset_y {
                cy -= state::BLOCK_I32;
            }

            let mut cur_fg = b'\0';
            let mut cur_bg = b'\0';

            let mut view_tracking = HashSet::new();
            for x in (cx..offset_x + width).step_by(state::BLOCK) {
                for y in (cy..offset_y + height).step_by(state::BLOCK) {
                    view_tracking.insert((x, y));
                    state.fill_block(x, y, &mut block);

                    for iy in 0..state::BLOCK_I32 {
                        let sy = y + iy - offset_y;
                        if sy < 0 || sy >= height {
                            continue;
                        }

                        let mut set_pos = false;
                        for ix in 0..state::BLOCK_I32 {
                            let sx = x + ix - offset_x;
                            if sx < 0 || sx >= width {
                                continue;
                            }

                            if !set_pos {
                                set_pos = true;
                                crossterm::queue!(
                                    stdout,
                                    crossterm::cursor::MoveTo(sx as u16, sy as u16),
                                )?;
                            }

                            let mut cursor = false;

                            for (cur_x, cur_y, _nick) in cursors.iter() {
                                let cur_x = cur_x - offset_x;
                                let cur_y = cur_y - offset_y;
                                if cur_x == sx && cur_y == sy {
                                    cursor = true;
                                    break;
                                }
                            }

                            let node = &block[ix as usize][iy as usize];

                            let mut want_fg = node.fg;
                            let mut want_bg = node.bg;

                            if cursor {
                                want_fg = state::BLACK as u32 as u8;
                                want_bg = state::WHITE as u32 as u8;
                            }

                            if cur_fg != want_fg {
                                cur_fg = want_fg;
                                crossterm::queue!(
                                    stdout,
                                    crossterm::style::SetForegroundColor(to_color(cur_fg, true)),
                                )?;
                            }

                            if cur_bg != want_bg {
                                cur_bg = want_bg;
                                crossterm::queue!(
                                    stdout,
                                    crossterm::style::SetBackgroundColor(to_color(cur_bg, false)),
                                )?;
                            }

                            crossterm::queue!(
                                stdout,
                                crossterm::style::Print(node.val),
                            )?;
                        }
                    }
                }
            }
            state.set_view_tracking(view_tracking);

            let title = format!("┫ textboard ┣━┫ {this_cur_x}x{this_cur_y} ┣");
            let sub = format!("┫ F1=fg F2=bg F3=reset ┣━┫   ┣");

            crossterm::queue!(
                stdout,
                crossterm::cursor::MoveTo(0, 0),
                crossterm::style::SetForegroundColor(crossterm::style::Color::Black),
                crossterm::style::SetBackgroundColor(crossterm::style::Color::White),
                crossterm::style::Print("━".repeat(width as usize)),
                crossterm::cursor::MoveTo(0, (height - 1) as u16),
                crossterm::style::Print("━".repeat(width as usize)),
                crossterm::cursor::MoveTo(2, 0),
                crossterm::style::Print(title),
                crossterm::cursor::MoveTo(2, (height - 1) as u16),
                crossterm::style::Print(sub),
                crossterm::style::SetForegroundColor(to_color(get_fg(), true)),
                crossterm::style::SetBackgroundColor(to_color(get_bg(), true)),
                crossterm::cursor::MoveTo(28, (height - 1) as u16),
                crossterm::style::Print(" * "),
                crossterm::style::ResetColor,
                crossterm::cursor::MoveTo(0, 0),
                crossterm::terminal::EndSynchronizedUpdate,
            )?;

            stdout.flush()?;
        }

        std::thread::sleep(std::time::Duration::from_millis(17));

        if state.should_exit() {
            break;
        }
    }

    Ok(event_task)
}
