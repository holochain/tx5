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

    let _l = Socket::with_v4(
        std::net::Ipv4Addr::UNSPECIFIED,
        Some(MULTICAST_V4),
        PORT,
        Arc::new(move |res| {
            let (data, addr) = res.unwrap();
            let s = String::from_utf8_lossy(&data);
            println!("{addr:?}: {s}");
        }),
    )
    .await
    .unwrap();

    let c = Socket::with_v4(
        std::net::Ipv4Addr::UNSPECIFIED,
        None,
        0,
        Arc::new(|_| ()),
    )
    .await
    .unwrap();

    loop {
        c.send(my_name.as_bytes().to_vec(), (MULTICAST_V4, PORT).into())
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }
}
