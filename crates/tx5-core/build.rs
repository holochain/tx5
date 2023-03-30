fn main() {
    println!("cargo:rerun-if-changed=src/README.tpl");
    let data = std::fs::read("src/README.tpl").expect("read README.tpl");
    let data = String::from_utf8_lossy(&data);
    let mut out = std::env::var("OUT_DIR")
        .map(std::path::PathBuf::from)
        .expect("read OUT_DIR");
    out.push("readme.rs");
    std::fs::write(
        out,
        format!(
            r##"
/// Internal unstable api to include the tx5 common doc header.
#[macro_export]
#[doc(hidden)]
macro_rules! __doc_header {{
    () => {{
        r#"{data}"#
    }};
}}
"##
        ),
    )
    .expect("write readme.rs");
}
