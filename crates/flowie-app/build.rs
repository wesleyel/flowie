// Record the git commit for the About panel.
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
}
