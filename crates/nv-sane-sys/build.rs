fn main() {
    // Look for libsane. Prefer pkg-config; fall back to manual search.
    // If only the versioned .so.1 exists (no -devel package), create a symlink
    // in OUT_DIR so the linker can resolve it.
    if try_pkg_config() {
        return;
    }
    if try_manual_link() {
        return;
    }
    panic!(
        "Could not locate libsane. Install sane-backends-libs (runtime) \
         or sane-backends-devel (development headers)."
    );
}

fn try_pkg_config() -> bool {
    // pkg-config is optional; skip gracefully if not found or not configured.
    let Ok(output) = std::process::Command::new("pkg-config").args(["--libs", "--silence-errors", "sane-backends"]).output() else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    // pkg-config gave us link flags. Pass on its search paths: a prefix that is not on the
    // linker's own path (/app in a Flatpak build) is otherwise not found.
    for flag in String::from_utf8_lossy(&output.stdout).split_whitespace() {
        if let Some(dir) = flag.strip_prefix("-L") {
            println!("cargo:rustc-link-search=native={dir}");
        }
    }
    println!("cargo:rustc-link-lib=sane");
    true
}

fn try_manual_link() -> bool {
    let search_dirs = ["/usr/lib64", "/usr/lib", "/usr/local/lib64", "/usr/local/lib"];

    for dir in search_dirs {
        let so = format!("{dir}/libsane.so");
        let so1 = format!("{dir}/libsane.so.1");

        if std::path::Path::new(&so).exists() {
            println!("cargo:rustc-link-search=native={dir}");
            println!("cargo:rustc-link-lib=sane");
            return true;
        }

        if std::path::Path::new(&so1).exists() {
            // -devel not installed: create a symlink in OUT_DIR so the linker
            // can find "libsane.so" without the unversioned file.
            let out = std::env::var("OUT_DIR").unwrap();
            let link = format!("{out}/libsane.so");
            if !std::path::Path::new(&link).exists() {
                std::os::unix::fs::symlink(&so1, &link).expect("failed to create libsane.so symlink in OUT_DIR");
            }
            println!("cargo:rustc-link-search=native={out}");
            println!("cargo:rustc-link-search=native={dir}");
            println!("cargo:rustc-link-lib=sane");
            return true;
        }
    }
    false
}
