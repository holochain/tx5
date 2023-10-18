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

    go_check_version();
    go_unzip_vendor();
    go_build();
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
        ver >= 1.20,
        "go executable version must be >= 1.20, got: {ver}",
    );
}

fn go_unzip_vendor() {
    //println!("cargo:warning=NOTE:go unzip vendor");

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

fn set_cmd_go_env(cmd: &mut Command, tgt_os: &str) {
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
    match tgt_os {
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
}

fn go_build() {
    let out_dir = std::env::var("OUT_DIR")
        .map(std::path::PathBuf::from)
        .expect("error reading out dir");

    let mut lib = out_dir.clone();

    #[cfg(not(windows))]
    lib.push("libgo-pion-webrtc.a");
    #[cfg(windows)]
    lib.push("go-pion-webrtc.lib");

    let mut lib_path = out_dir.clone();

    let tgt_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap();

    match tgt_os.as_str() {
        "macos" | "ios" | "tvos" => {
            lib_path.push("go-pion-webrtc.dylib");
        }
        "windows" => {
            lib_path.push("go-pion-webrtc.dll");
        }
        _ => {
            lib_path.push("go-pion-webrtc.so");
        }
    }

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

    cp("buffer.go");
    cp("const.go");
    cp("datachannel.go");
    cp("main.go");
    cp("peerconnection.go");
    cp("go.sum");
    cp("go.mod");

    let mut cmd = Command::new("go");

    set_cmd_go_env(&mut cmd, tgt_os.as_str());

    // grr, clippy, the debug symbols belong in one arg
    #[allow(clippy::suspicious_command_arg_space)]
    {
        cmd.current_dir(out_dir.clone())
            .env("GOCACHE", cache.clone())
            .arg("build")
            .arg("-ldflags") // strip debug symbols
            .arg("-s -w") // strip debug symbols
            .arg("-buildvcs=false") // disable version control stamping binary
            .arg("-o")
            .arg(lib_path.clone())
            .arg("-mod=vendor")
            .arg("-buildmode=c-shared");
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

    #[cfg(not(windows))]
    {
        let mut cmd = Command::new("go");

        set_cmd_go_env(&mut cmd, tgt_os.as_str());

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
                .arg(lib)
                .arg("-mod=vendor")
                .arg("-buildmode=c-archive");
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

        println!(
            "cargo:rustc-link-search=native={}",
            out_dir.to_string_lossy()
        );
        println!("cargo:rustc-link-lib=static=go-pion-webrtc");
    }

    use sha2::Digest;
    let data = std::fs::read(lib_path).expect("failed to read generated lib");
    let mut hasher = sha2::Sha256::new();
    hasher.update(data);
    let hash =
        base64::encode_config(hasher.finalize(), base64::URL_SAFE_NO_PAD);

    let mut exe_hash = out_dir;
    exe_hash.push("lib_hash.rs");
    std::fs::write(
        exe_hash,
        format!(
            r#"
        const LIB_HASH: &str = "{hash}";
    "#,
        ),
    )
    .expect("failed to write lib hash");
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
            out_lines.push(format!("/{line}"));
        } else if line.starts_with("Ty") {
            let mut ws = line.split_whitespace();
            let id = ws.next().unwrap();
            let id = (*id).to_screaming_snake_case();
            ws.next().unwrap(); //ty
            ws.next().unwrap(); //=
            let val = ws.next().unwrap();
            out_lines.push(format!("pub const {id}: usize = {val};"));
        }

        let mut out: std::path::PathBuf =
            std::env::var_os("OUT_DIR").unwrap().into();
        out.push("constants.rs");
        std::fs::write(&out, out_lines.join("\n")).unwrap();
    }
}
