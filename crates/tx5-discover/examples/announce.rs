use std::sync::Arc;
use tx5_discover::*;

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let my_name = format!(
        "tx5-discover-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_micros()
            % 9931
    );
    println!("my_name: {my_name}");

    let sg = Shotgun::new(
        Arc::new(move |res| {
            let (data, addr) = res.unwrap();
            let s = String::from_utf8_lossy(&data);
            println!("{addr:?}: {s}");
        }),
        PORT,
        MULTICAST_V4,
        MULTICAST_V6,
    )
    .await
    .unwrap();

    loop {
        sg.multicast(my_name.as_bytes().to_vec()).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }
}
