#![forbid(unsafe_code)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let compiler = std::process::Command::new(std::env::var_os("RUSTC").ok_or("RUSTC is not set")?)
        .arg("--version")
        .output()?;
    if !compiler.status.success() {
        return Err("Cannot identify Rust compiler".into());
    }
    let version = String::from_utf8(compiler.stdout)?;
    // Cargo build metadata protocol, not production logging.
    println!("cargo:rustc-env=FLY_TRACKER_COMPILER={}", version.trim());
    println!(
        "cargo:rustc-env=FLY_TRACKER_TARGET={}",
        std::env::var("TARGET")?
    );
    println!("cargo:rerun-if-env-changed=RUSTC");
    Ok(())
}
