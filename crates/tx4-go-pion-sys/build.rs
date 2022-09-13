use std::process::Command;

fn main() {
    //println!("cargo:warning=NOTE:running go-pion-webrtc-sys build.rs");

    println!("cargo:rerun-if-changed=go.mod");
    println!("cargo:rerun-if-changed=go.sum");
    println!("cargo:rerun-if-changed=buffer.go");
    println!("cargo:rerun-if-changed=const.go");
    println!("cargo:rerun-if-changed=datachannel.go");
    println!("cargo:rerun-if-changed=peerconnection.go");
    println!("cargo:rerun-if-changed=main.go");
    println!("cargo:rerun-if-changed=vendor.zip");

    let mut lib_path = std::path::PathBuf::from(
        std::env::var("OUT_DIR").expect("failed to read env OUT_DIR"),
    );
    #[cfg(target_os = "macos")]
    lib_path.push("go-pion-webrtc.dylib");
    #[cfg(target_os = "windows")]
    lib_path.push("go-pion-webrtc.dll");
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    lib_path.push("go-pion-webrtc.so");

    go_check_version();
    go_unzip_vendor();
    go_build(&lib_path);
    gen_rust_const();
}

fn go_check_version() {
    //println!("cargo:warning=NOTE:checking go version");

    let go_version = Command::new("go")
        .arg("version")
        .output()
        .expect("error checking go version");
    assert_eq!(b"go version go", &go_version.stdout[0..13]);
    let ver: f64 = String::from_utf8_lossy(&go_version.stdout[13..17])
        .parse()
        .expect("error parsing go version");
    assert!(
        ver >= 1.19,
        "go executable version must be >= 1.19, got: {}",
        ver
    );
}

fn go_unzip_vendor() {
    //println!("cargo:warning=NOTE:go unzip vendor");

    let manifest_path = std::env::var("CARGO_MANIFEST_DIR")
        .map(std::path::PathBuf::from)
        .expect("error reading manifest dir");

    let mut vendor_path = manifest_path.clone();
    vendor_path.push("vendor.zip");

    zip::read::ZipArchive::new(
        std::fs::File::open(vendor_path)
            .expect("failed to open vendor zip file"),
    )
    .expect("failed to open vendor zip file")
    .extract(manifest_path)
    .expect("failed to extract vendor zip file");
}

fn go_build(path: &std::path::Path) {
    let mut cmd = Command::new("go");
    cmd.arg("build")
        .arg("-o")
        .arg(path)
        .arg("-mod=vendor")
        .arg("-buildmode=c-shared");

    println!("cargo:warning=NOTE:running go build: {:?}", cmd);

    assert!(
        cmd.spawn()
            .expect("error spawing go build")
            .wait()
            .expect("error running go build")
            .success(),
        "error running go build",
    );
}

fn gen_rust_const() {
    use inflector::Inflector;
    use std::io::BufRead;

    let mut out_lines = Vec::new();

    for line in
        std::io::BufReader::new(std::fs::File::open("const.go").unwrap())
            .lines()
    {
        let line = line.unwrap();
        let line = line.trim();
        if line.starts_with("//") {
            out_lines.push(format!("/{}", line));
        } else if line.starts_with("Ty") {
            let mut ws = line.split_whitespace();
            let id = ws.next().unwrap();
            let id = (*id).to_screaming_snake_case();
            ws.next().unwrap(); //ty
            ws.next().unwrap(); //=
            let val = ws.next().unwrap();
            out_lines.push(format!("pub const {}: usize = {};", id, val));
        }

        let mut out: std::path::PathBuf =
            std::env::var_os("OUT_DIR").unwrap().into();
        out.push("constants.rs");
        std::fs::write(&out, out_lines.join("\n")).unwrap();
    }
}
