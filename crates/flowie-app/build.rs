// - Record the git commit for the About panel.
// - Embed every SVG in assets/icons (Lucide, ISC license) as `icons/<name>.svg`.
// - Embed every locales/<code>.json (English first: the source and fallback).
use std::fmt::Write;

fn main() {
    let out = std::process::Command::new("git").args(["rev-parse", "--short", "HEAD"]).output();
    if let Ok(o) = out {
        if o.status.success() {
            let hash = String::from_utf8_lossy(&o.stdout).trim().to_string();
            println!("cargo:rustc-env=FLOWIE_COMMIT={hash}");
        }
    }
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/refs/heads");

    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/icons");
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .expect("assets/icons")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.ends_with(".svg"))
        .collect();
    names.sort();
    let mut src = String::from("pub static ICONS: &[(&str, &[u8])] = &[\n");
    for n in &names {
        let path = dir.join(n);
        writeln!(src, "    (\"icons/{n}\", include_bytes!({:?})),", path.to_string_lossy()).unwrap();
    }
    src.push_str("];\n");
    let out = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("icons.rs");
    std::fs::write(out, src).unwrap();

    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("locales");
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut codes: Vec<String> = std::fs::read_dir(&dir)
        .expect("locales")
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().to_string_lossy().strip_suffix(".json").map(String::from))
        .collect();
    codes.sort_by_key(|c| (c != "en", c.clone()));
    assert_eq!(codes.first().map(String::as_str), Some("en"), "locales/en.json is required");
    let mut src = String::from("pub static LOCALES: &[(&str, &str)] = &[\n");
    for c in &codes {
        let path = dir.join(format!("{c}.json"));
        writeln!(src, "    (\"{c}\", include_str!({:?})),", path.to_string_lossy()).unwrap();
    }
    src.push_str("];\n");
    let out = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("locales.rs");
    std::fs::write(out, src).unwrap();

    #[cfg(target_os = "windows")]
    {
        let ico = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/AppIcon.ico");
        println!("cargo:rerun-if-changed={}", ico.display());
        let mut res = winresource::WindowsResource::new();
        // GPUI's Windows backend loads resource ID 1 via LoadImageW(..., PCWSTR(1 as _), IMAGE_ICON, ...).
        res.set_icon_with_id(&ico.to_string_lossy(), "1");
        res.set("ProductName", "Flowie");
        res.set("FileDescription", "Flowie — a fast outliner for thinking in lists");
        res.compile().expect("embed Windows resources");
    }
}
