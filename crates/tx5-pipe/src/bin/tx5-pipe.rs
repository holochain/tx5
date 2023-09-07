#![deny(missing_docs)]
#![deny(warnings)]
//! tx5-pipe binary.

use bytes::*;
use std::sync::Arc;
use tx5_core::pipe_ipc::*;

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let snd = write_stdout();
    let mut rcv = read_stdin();

    let quit_not = Arc::new(tokio::sync::Notify::new());

    struct Handler(
        tokio::sync::mpsc::UnboundedSender<Tx5PipeResponse>,
        Arc<tokio::sync::Notify>,
    );

    impl tx5_pipe::Tx5PipeHandler for Handler {
        fn fatal(&self, error: std::io::Error) {
            eprintln!("Fatal: {error:?}");
            std::process::exit(127);
        }

        fn quit(&self) {
            self.1.notify_waiters();
        }

        fn pipe(&self, response: Tx5PipeResponse) -> bool {
            self.0.send(response).is_ok()
        }
    }

    let hnd = Handler(snd, quit_not.clone());

    let pipe = match tx5_pipe::Tx5Pipe::new(hnd).await {
        Err(err) => {
            eprintln!("Constructor: {err:?}");
            std::process::exit(127);
        }
        Ok(pipe) => pipe,
    };

    {
        let pipe = pipe.clone();
        tokio::task::spawn(async move {
            quit_not.notified().await;
            let _ = pipe.shutdown().await;
            eprintln!("Exiting on Quit");
            std::process::exit(0);
        });
    }

    {
        let pipe = pipe.clone();
        tokio::task::spawn(async move {
            let _ = tokio::signal::ctrl_c().await;
            let _ = pipe.shutdown().await;
            eprintln!("Exiting on Ctrl-C");
            std::process::exit(0);
        });
    }

    while let Some(req) = rcv.recv().await {
        pipe.pipe(req);
    }
}

fn write_stdout() -> tokio::sync::mpsc::UnboundedSender<Tx5PipeResponse> {
    use std::io::Write;

    let (send, mut recv) =
        tokio::sync::mpsc::unbounded_channel::<Tx5PipeResponse>();

    std::thread::spawn(move || {
        let stdout = std::io::stdout();
        let mut stdout = stdout.lock();

        let mut enc = asv::AsvEncoder::default();

        while let Some(res) = recv.blocking_recv() {
            if let Err(err) = res.encode(&mut enc) {
                eprintln!("Response Encode: {err:?}");
                std::process::exit(127);
            }

            while let Ok(res) = recv.try_recv() {
                if let Err(err) = res.encode(&mut enc) {
                    eprintln!("Response Encode: {err:?}");
                    std::process::exit(127);
                }
            }

            let mut buf = enc.drain();
            while buf.has_remaining() {
                let c = buf.chunk();
                if let Err(err) = stdout.write_all(c) {
                    eprintln!("Stdout Write: {err:?}");
                    std::process::exit(127);
                }
                buf.advance(c.len());
            }
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

        let mut dec = asv::AsvParser::default();

        const LOW_CAP: usize = 1024;
        const HIGH_CAP: usize = 8 * LOW_CAP;

        let mut buf = BytesMut::with_capacity(HIGH_CAP);

        loop {
            if buf.capacity() < LOW_CAP {
                std::mem::swap(
                    &mut buf,
                    &mut BytesMut::with_capacity(HIGH_CAP),
                );
            }
            // unsafe read until read_buf is stablized
            unsafe {
                buf.set_len(buf.capacity());
            }
            match stdin.read(&mut buf) {
                Ok(read) => {
                    unsafe {
                        buf.set_len(read);
                    }
                    if buf.has_remaining() {
                        let frozen = buf.split_to(buf.len()).freeze();
                        let res = match dec.parse(frozen) {
                            Err(err) => {
                                eprintln!("Request Decode1: {err:?}");
                                std::process::exit(127);
                            }
                            Ok(res) => res,
                        };
                        for field_list in res {
                            let req = match Tx5PipeRequest::decode(field_list) {
                                Err(err) => {
                                    eprintln!("Request Decode2: {err:?}");
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
                    unsafe {
                        buf.set_len(0);
                    }
                    eprintln!("Stdin Read: {err:?}");
                    std::process::exit(127);
                }
            }
        }
    });
    recv
}
