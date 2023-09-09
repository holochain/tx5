use std::sync::{Arc, Mutex};

mod state;
use bytes::*;
use state::*;

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
            //crossterm::terminal::DisableLineWrap,
            crossterm::cursor::Hide,
        )?;
    }

    let (width, height) = crossterm::terminal::size()?;

    let mut offset_x: i32 = width as i32 / -2;
    let mut offset_y: i32 = height as i32 / -2;

    let event_state = state.clone();
    let event_task =
        tokio::task::spawn(tokio::task::spawn_blocking(move || {
            run_event(event_state)
        }));

    let mut block = [[state::Node::default(); state::BLOCK]; state::BLOCK];

    loop {
        {
            let (width, height) = crossterm::terminal::size()?;

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

            for x in (cx..offset_x + width as i32).step_by(state::BLOCK) {
                for y in (cy..offset_y + height as i32).step_by(state::BLOCK) {
                    /*
                    crossterm::queue!(
                        stdout,
                        crossterm::style::Print(format!("(B{x},{y})")),
                    )?;
                    */

                    state.fill_block(x, y, &mut block);

                    for iy in 0..state::BLOCK_I32 {
                        if iy - offset_y < 0 ||
                            iy - offset_y > height as i32
                        {
                            continue;
                        }
                        let mut set_pos = false;
                        for ix in 0..state::BLOCK_I32 {
                            if ix - offset_x < 0 ||
                                ix - offset_x > width as i32
                            {
                                continue;
                            }
                            if !set_pos {
                                set_pos = true;
                                crossterm::queue!(
                                    stdout,
                                    crossterm::cursor::MoveTo(ix as u16, iy as u16),
                                )?;
                            }
                            crossterm::queue!(
                                stdout,
                                crossterm::style::Print(block[ix as usize][iy as usize].val),
                            )?;
                            /*
                            crossterm::queue!(
                                stdout,
                                crossterm::style::Print(format!("(I{ix},{iy})")),
                            )?;
                            */
                        }
                    }
                }
            }

            crossterm::queue!(
                stdout,
                crossterm::cursor::MoveTo(0, 0),
                crossterm::style::SetForegroundColor(crossterm::style::Color::Black),
                crossterm::style::SetBackgroundColor(crossterm::style::Color::White),
                crossterm::style::Print("━".repeat(width as usize)),
                crossterm::cursor::MoveTo(0, height - 1),
                crossterm::style::Print("━".repeat(width as usize)),
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
