fn main() {
    // Emit the application ID, set by Meson through the environment.
    // Falls back to the release ID when building with plain `cargo build`.
    let app_id = std::env::var("APP_ID")
        .unwrap_or_else(|_| "io.github.cschalle.NegativeVibes".to_owned());
    println!("cargo:rustc-env=APP_ID={app_id}");

    let pkgdatadir = std::env::var("PKGDATADIR").unwrap_or_else(|_| {
        // For plain cargo builds, look in the source tree's data/ directory.
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        format!("{manifest_dir}/../data")
    });
    println!("cargo:rustc-env=PKGDATADIR={pkgdatadir}");

    println!("cargo:rerun-if-env-changed=APP_ID");
    println!("cargo:rerun-if-env-changed=PKGDATADIR");
}
