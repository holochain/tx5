#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let mut r = tx5_online::OnlineReceiver::new();
    println!("{}", r.status());
    loop {
        println!("{}", r.recv().await);
    }
}
