// Compile FlowieCloudKit's Swift sources and the C bridge into a static library and link it
// with the Swift runtime and CloudKit (macOS only). Needs `swiftc` (the Xcode Command Line
// Tools are enough). Without it, or with FLOWIE_CLOUDKIT=0, the crate builds without iCloud.
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo::rustc-check-cfg=cfg(cloudkit_swift)");
    println!("cargo:rerun-if-env-changed=FLOWIE_CLOUDKIT");
    println!("cargo:rerun-if-env-changed=MACOSX_DEPLOYMENT_TARGET");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    if std::env::var("FLOWIE_CLOUDKIT").as_deref() == Ok("0") {
        println!("cargo:warning=FLOWIE_CLOUDKIT=0: building without iCloud sync");
        return;
    }
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let shared = manifest.join("../../apple/FlowieKit/Sources/FlowieCloudKit");
    let bridge = manifest.join("swift/Bridge.swift");
    let mut sources = swift_files(&shared);
    sources.push(bridge.clone());
    println!("cargo:rerun-if-changed={}", shared.display());
    for source in &sources {
        println!("cargo:rerun-if-changed={}", source.display());
    }

    let arch = match std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("aarch64") => "arm64",
        Ok("x86_64") => "x86_64",
        other => panic!("flowie-cloudkit: no Swift target for {other:?}"),
    };
    let min = std::env::var("MACOSX_DEPLOYMENT_TARGET").unwrap_or_else(|_| "13.0".into());
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let lib = out.join("libflowie_cloudkit_swift.a");
    let release = std::env::var("PROFILE").as_deref() == Ok("release");
    let status = Command::new("xcrun")
        .args(["swiftc", "-parse-as-library", "-emit-library", "-static", "-swift-version", "5"])
        .args(["-module-name", "FlowieCloudKit", "-target", &format!("{arch}-apple-macos{min}")])
        .args(["-module-cache-path", &out.join("module-cache").to_string_lossy()])
        .arg(if release { "-O" } else { "-Onone" })
        .arg("-o")
        .arg(&lib)
        .args(&sources)
        .status();
    match status {
        Ok(s) if s.success() => {}
        Ok(s) => panic!("flowie-cloudkit: swiftc failed ({s}); set FLOWIE_CLOUDKIT=0 to build without iCloud"),
        Err(e) => {
            println!("cargo:warning=no Swift compiler ({e}): building without iCloud sync");
            return;
        }
    }

    println!("cargo:rustc-cfg=cloudkit_swift");
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=flowie_cloudkit_swift");
    for framework in ["CloudKit", "Foundation", "Security"] {
        println!("cargo:rustc-link-lib=framework={framework}");
    }
    // The Swift runtime is part of macOS (/usr/lib/swift); the toolchain has the small
    // compatibility libraries the objects ask for when built for an older macOS.
    let sdk = output(&["--sdk", "macosx", "--show-sdk-path"]);
    println!("cargo:rustc-link-search=native={sdk}/usr/lib/swift");
    let swiftc = output(&["--find", "swiftc"]);
    let toolchain = Path::new(&swiftc).parent().and_then(Path::parent).expect("swiftc in <toolchain>/usr/bin");
    println!("cargo:rustc-link-search=native={}", toolchain.join("lib/swift/macosx").display());
    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
}

fn swift_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("flowie-cloudkit: {}: {e}", dir.display()))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "swift"))
        .collect();
    files.sort();
    files
}

fn output(args: &[&str]) -> String {
    let out = Command::new("xcrun").args(args).output().expect("xcrun");
    String::from_utf8(out.stdout).expect("utf-8").trim().to_string()
}
