// rust-embed needs the folder to exist at compile time, even when the web UI
// has not been built yet (plain `cargo build`, `cargo check`, `cargo test`).
fn main() {
    let _ = std::fs::create_dir_all("web/dist");
    println!("cargo:rerun-if-changed=build.rs");
}
