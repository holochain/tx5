mod state;
use bytes::*;
use state::*;

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    struct PHnd;

    impl tx5_pipe_control::Tx5PipeControlHandler for PHnd {
        fn recv(&self, _: String, _: Box<(dyn Buf + Send + 'static)>) {
            todo!()
        }
    }

    let pipe = tx5_pipe::Tx5Pipe::spawn_in_process(PHnd).await.unwrap();

    let cli_url = pipe
        .sig_reg("wss://signal.holo.host".to_string())
        .await
        .unwrap();

    struct SHnd {
        pipe: tx5_pipe_control::Tx5PipeControl,
    }

    impl StateHnd for SHnd {
        fn prompt(&self, p: String) -> BoxFut<String> {
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

        fn show_board(&self) {
            println!("show board");
            std::process::exit(0);
        }
    }

    let state = State::new(SHnd { pipe }, cli_url).await;

    println!("yo: {state:?}");

    state.wait_exit().await;
}
