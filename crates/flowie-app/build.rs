// - Record the git commit for the About panel.
// - Embed every SVG in assets/icons (Lucide, ISC license) as `icons/<name>.svg`.
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
        writeln!(src, "    (\"icons/{n}\", include_bytes!(\"{}\")),", dir.join(n).display()).unwrap();
    }
    src.push_str("];\n");
    let out = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("icons.rs");
    std::fs::write(out, src).unwrap();
}
