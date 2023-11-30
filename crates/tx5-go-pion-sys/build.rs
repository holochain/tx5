use std::{path::Path, process::Command};

#[derive(Debug)]
enum LinkType {
    Dynamic,
    Static,
}

#[derive(Debug)]
struct Target {
    pub go_arch: &'static str,
    pub go_os: &'static str,
    pub link_type: LinkType,
}

impl Default for Target {
    fn default() -> Self {
        let target_arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap();

        let go_arch = match target_arch.as_str() {
            "arm" => "arm",
            "aarch64" => "arm64",
            "x86_64" => "amd64",
            "x86" => "386",
            oth => panic!("{oth} arch not yet supported"),
        };

        let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap();

        let go_os = match target_os.as_str() {
            "windows" => "windows",
            "macos" => "darwin",
            "ios" => "ios",
            "linux" => "linux",
            "android" => "android",
            "dragonfly" => "dragonfly",
            "freebsd" => "freebsd",
            "openbsd" => "openbsd",
            "netbsd" => "netbsd",
            oth => panic!("{oth} os not yet supported"),
        };

        let mut link_type = match target_os.as_str() {
            "windows" | "android" => LinkType::Dynamic,
            _ => LinkType::Static,
        };

        let mut link_type_forced = false;

        if std::env::var("CARGO_FEATURE_FORCE_DYNAMIC_LINK").is_ok() {
            link_type = LinkType::Dynamic;
            link_type_forced = true;
        }

        if std::env::var("CARGO_FEATURE_FORCE_STATIC_LINK").is_ok() {
            if link_type_forced {
                panic!("force_static_link and force_dynamic_link cannot both be specified");
            }
            link_type = LinkType::Static;
        }

        match link_type {
            LinkType::Dynamic => println!("cargo:rustc-cfg=link_dynamic"),
            LinkType::Static => println!("cargo:rustc-cfg=link_static"),
        }

        Self {
            go_arch,
            go_os,
            link_type,
        }
    }
}

static TARGET: once_cell::sync::Lazy<Target> =
    once_cell::sync::Lazy::new(Target::default);

fn main() {
    //println!("cargo:warning={:?}", *TARGET);

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

fn go_build_cmd(
    out_dir: &std::path::Path,
    lib_path: &std::path::Path,
    build_mode: &str,
) {
    let mut cache = std::path::PathBuf::from(out_dir);
    cache.push("go-build");

    let mut cmd = Command::new("go");

    cmd.env("GOARCH", TARGET.go_arch);

    cmd.env("GOOS", TARGET.go_os);

    if let Ok(linker) = std::env::var("RUSTC_LINKER") {
        //println!("cargo:warning=LINKER: {linker:?}");
        cmd.env("CC_FOR_TARGET", &linker);
        cmd.env("CC", &linker);
    }

    cmd.env("CGO_ENABLED", "1");

    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap();
    if target_os == "ios" {
        // Determine Xcode directory path
        let xcode_select_output =
            Command::new("xcode-select").arg("-p").output().unwrap();
        if !xcode_select_output.status.success() {
            panic!("Failed to run xcode-select -p");
        }
        let xcode_dir = String::from_utf8(xcode_select_output.stdout)
            .unwrap()
            .trim()
            .to_string();

        // Determine SDK directory paths
        let sdk_dir_ios = Path::new(&xcode_dir)
            .join("Platforms/iPhoneOS.platform/Developer/SDKs/iPhoneOS.sdk")
            .to_str()
            .unwrap()
            .to_string();

        cmd.env("CGO_CFLAGS", format!(" -isysroot {sdk_dir_ios}"));
    }

    // grr, clippy, the debug symbols belong in one arg
    #[allow(clippy::suspicious_command_arg_space)]
    {
        cmd.current_dir(out_dir)
            .env("GOCACHE", cache)
            .arg("build")
            .arg("-ldflags") // strip debug symbols
            .arg("-s -w") // strip debug symbols
            .arg("-buildvcs=false") // disable version control stamping binary
            .arg("-o")
            .arg(lib_path)
            .arg("-mod=vendor")
            .arg(build_mode);
    }

    //println!("cargo:warning=NOTE:running go build: {cmd:?}");

    assert!(
        cmd.spawn()
            .expect("error spawing go build")
            .wait()
            .expect("error running go build")
            .success(),
        "error running go build",
    );
}

fn go_build() {
    let out_dir = std::env::var("OUT_DIR")
        .map(std::path::PathBuf::from)
        .expect("error reading out dir");

    let manifest_path = std::env::var("CARGO_MANIFEST_DIR")
        .map(std::path::PathBuf::from)
        .expect("error reading manifest dir");

    let cp = |f: &'static str| {
        let mut a = manifest_path.clone();
        a.push(f);
        let mut b = out_dir.clone();
        b.push(f);
        std::fs::copy(a, b).expect("failed to copy golang project file");
    };

    cp("buffer.go");
    cp("const.go");
    cp("datachannel.go");
    cp("main.go");
    cp("peerconnection.go");
    cp("go.sum");
    cp("go.mod");

    match TARGET.link_type {
        LinkType::Dynamic => {
            let mut lib_path = out_dir.clone();

            match TARGET.go_os {
                "darwin" | "ios" => {
                    lib_path.push("go-pion-webrtc.dylib");
                }
                "windows" => {
                    lib_path.push("go-pion-webrtc.dll");
                }
                _ => {
                    lib_path.push("go-pion-webrtc.so");
                }
            }

            go_build_cmd(&out_dir, &lib_path, "-buildmode=c-shared");

            use sha2::Digest;
            let data =
                std::fs::read(lib_path).expect("failed to read generated lib");
            let mut hasher = sha2::Sha256::new();
            hasher.update(data);
            let hash = base64::encode_config(
                hasher.finalize(),
                base64::URL_SAFE_NO_PAD,
            );

            let mut lib_hash = out_dir;
            lib_hash.push("lib_hash.rs");
            std::fs::write(
                lib_hash,
                format!("const LIB_HASH: &str = \"{hash}\";\n"),
            )
            .expect("failed to write lib hash");
        }
        LinkType::Static => {
            let mut lib_path = out_dir.clone();

            if let "windows" = TARGET.go_os {
                lib_path.push("go-pion-webrtc.lib");
            } else {
                lib_path.push("libgo-pion-webrtc.a");
            }

            go_build_cmd(&out_dir, &lib_path, "-buildmode=c-archive");

            println!(
                "cargo:rustc-link-search=native={}",
                out_dir.to_string_lossy()
            );
            println!("cargo:rustc-link-lib=static=go-pion-webrtc");
        }
    }
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
