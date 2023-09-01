#![deny(missing_docs)]
#![deny(unsafe_code)]
#![deny(warnings)]
//! tx5-pipe binary.

use tx5_core::pipe_ipc::*;

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let snd = write_stdout();
    let mut rcv = read_stdin();

    struct Handler(tokio::sync::mpsc::UnboundedSender<Tx5PipeResponse>);

    impl tx5_pipe::server::Tx5PipeServerHandler for Handler {
        fn fatal(&self, error: std::io::Error) {
            eprintln!("{error:?}");
            std::process::exit(127);
        }

        fn pipe(&self, response: Tx5PipeResponse) -> bool {
            self.0.send(response).is_ok()
        }
    }

    let hnd = Handler(snd);

    let pipe = match tx5_pipe::server::Tx5PipeServer::new(hnd).await {
        Err(err) => {
            eprintln!("{err:?}");
            std::process::exit(127);
        }
        Ok(pipe) => pipe,
    };

    while let Some(req) = rcv.recv().await {
        pipe.pipe(req);
    }
}

fn write_stdout() -> tokio::sync::mpsc::UnboundedSender<Tx5PipeResponse> {
    let (send, mut recv) =
        tokio::sync::mpsc::unbounded_channel::<Tx5PipeResponse>();
    std::thread::spawn(move || {
        let stdout = std::io::stdout();
        let mut stdout = stdout.lock();

        while let Some(res) = recv.blocking_recv() {
            match res.encode(&mut stdout) {
                Err(err) => {
                    eprintln!("{err:?}");
                    std::process::exit(127);
                }
                Ok(enc) => enc,
            };
        }
    });

    send
}

fn read_stdin() -> tokio::sync::mpsc::UnboundedReceiver<Tx5PipeRequest> {
    use std::io::Read;

    let (send, recv) = tokio::sync::mpsc::unbounded_channel();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        let mut stdin = stdin.lock();

        let mut dec = asv::AsvParse::default();
        let mut buf = [0; 4096];

        loop {
            match stdin.read(&mut buf) {
                Ok(read) => {
                    if read > 0 {
                        let res = match dec.parse(&buf[..read]) {
                            Err(err) => {
                                eprintln!("{err:?}");
                                std::process::exit(127);
                            }
                            Ok(res) => res,
                        };
                        for field_list in res {
                            let req = match Tx5PipeRequest::decode(field_list) {
                                Err(err) => {
                                    eprintln!("{err:?}");
                                    std::process::exit(127);
                                }
                                Ok(req) => req,
                            };
                            if send.send(req).is_err() {
                                eprintln!("stdin send channel closed");
                                std::process::exit(127);
                            }
                        }
                    }
                }
                Err(err) => {
                    eprintln!("{err:?}");
                    std::process::exit(127);
                }
            }
        }
    });
    recv
}
