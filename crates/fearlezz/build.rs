//! Computes what the binary ships (`src/shipped.rs`) into `FEARLEZZ_SHIPPED`.

#[path = "src/shipped.rs"]
mod shipped;

use std::path::PathBuf;

fn main() -> std::io::Result<()> {
    let crate_dir = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap_or_default());
    for source in shipped::SOURCES {
        println!(
            "cargo::rerun-if-changed={}",
            crate_dir.join(source).display()
        );
    }
    println!(
        "cargo::rustc-env=FEARLEZZ_SHIPPED={}",
        shipped::digest(&crate_dir)?
    );
    Ok(())
}
