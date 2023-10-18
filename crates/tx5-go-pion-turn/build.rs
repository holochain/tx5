use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=go.mod");
    println!("cargo:rerun-if-changed=go.sum");
    println!("cargo:rerun-if-changed=main.go");
    println!("cargo:rerun-if-changed=vendor.zip");

    let mut bin_path = std::path::PathBuf::from(
        std::env::var("OUT_DIR").expect("failed to read env OUT_DIR"),
    );
    bin_path.push("tx5-go-pion-turn");

    go_check_version();
    go_unzip_vendor();
    go_build(&bin_path);
}

fn go_check_version() {
    let go_version = Command::new("go")
        .arg("version")
        .output()
        .expect("error checking go version");
    assert_eq!(b"go version go", &go_version.stdout[0..13]);
    let ver: f64 = String::from_utf8_lossy(&go_version.stdout[13..17])
        .parse()
        .expect("error parsing go version");
    assert!(
        ver >= 1.18,
        "go executable version must be >= 1.18, got: {ver}",
    );
}

fn go_unzip_vendor() {
    let out_dir = std::env::var("OUT_DIR")
        .map(std::path::PathBuf::from)
        .expect("error reading out dir");

    let mut vendor_path = std::env::var("CARGO_MANIFEST_DIR")
        .map(std::path::PathBuf::from)
        .expect("error reading manifest dir");
    vendor_path.push("vendor.zip");

    zip::read::ZipArchive::new(
        std::fs::File::open(vendor_path)
            .expect("failed to open vendor zip file"),
    )
    .expect("failed to open vendor zip file")
    .extract(out_dir)
    .expect("failed to extract vendor zip file");
}

fn go_build(path: &std::path::Path) {
    let out_dir = std::env::var("OUT_DIR")
        .map(std::path::PathBuf::from)
        .expect("error reading out dir");

    let mut cache = out_dir.clone();
    cache.push("go-build");

    let manifest_path = std::env::var("CARGO_MANIFEST_DIR")
        .map(std::path::PathBuf::from)
        .expect("error reading manifest dir");

    let cp = |f: &'static str| {
        let mut a = manifest_path.clone();
        a.push(f);
        let mut b = out_dir.clone();
        b.push(f);
        std::fs::copy(a, b).expect("failed to copy go file");
    };

    cp("main.go");
    cp("go.sum");
    cp("go.mod");

    let mut cmd = Command::new("go");

    // add some cross-compilation translators:
    match std::env::var("CARGO_CFG_TARGET_ARCH").unwrap().as_str() {
        "arm" => {
            cmd.env("GOARCH", "arm");
        }
        "aarch64" => {
            cmd.env("GOARCH", "arm64");
        }
        "x86_64" => {
            cmd.env("GOARCH", "amd64");
        }
        "x86" => {
            cmd.env("GOARCH", "386");
        }
        _ => (),
    }

    // and for the os
    let tgt_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap();
    match tgt_os.as_str() {
        "windows" => {
            cmd.env("GOOS", "windows");
        }
        "macos" => {
            cmd.env("GOOS", "darwin");
        }
        "ios" => {
            cmd.env("GOOS", "ios");
        }
        "linux" => {
            cmd.env("GOOS", "linux");
        }
        "android" => {
            cmd.env("GOOS", "android");
        }
        "dragonfly" => {
            cmd.env("GOOS", "dragonfly");
        }
        "freebsd" => {
            cmd.env("GOOS", "freebsd");
        }
        "openbsd" => {
            cmd.env("GOOS", "openbsd");
        }
        "netbsd" => {
            cmd.env("GOOS", "netbsd");
        }
        _ => (),
    }

    if tgt_os == "android" {
        let linker = std::env::var("RUSTC_LINKER").unwrap();
        println!("cargo:warning=LINKER: {linker:?}");
        cmd.env("CC_FOR_TARGET", &linker);
        cmd.env("CC", &linker);
        cmd.env("CGO_ENABLED", "1");
    }

    // grr, clippy, the debug symbols belong in one arg
    #[allow(clippy::suspicious_command_arg_space)]
    {
        cmd.current_dir(out_dir.clone())
            .env("GOCACHE", cache)
            .arg("build")
            .arg("-ldflags") // strip debug symbols
            .arg("-s -w") // strip debug symbols
            .arg("-buildvcs=false") // disable version control stamping binary
            .arg("-o")
            .arg(path)
            .arg("-mod=vendor");
    }

    println!("cargo:warning=NOTE:running go build: {cmd:?}");

    assert!(
        cmd.spawn()
            .expect("error spawing go build")
            .wait()
            .expect("error running go build")
            .success(),
        "error running go build",
    );

    use sha2::Digest;
    let data = std::fs::read(path).expect("failed to read generated exe");
    let mut hasher = sha2::Sha256::new();
    hasher.update(data);
    let hash =
        base64::encode_config(hasher.finalize(), base64::URL_SAFE_NO_PAD);

    let mut exe_hash = out_dir;
    exe_hash.push("exe_hash.rs");
    std::fs::write(
        exe_hash,
        format!(
            r#"
        const EXE_HASH: &str = "{hash}";
    "#,
        ),
    )
    .expect("failed to write exe hash");
}
