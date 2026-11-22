fn main() {
    println!("cargo:rerun-if-changed=ui");
    println!("cargo:rerun-if-changed=ui/app.js");
    println!("cargo:rerun-if-changed=ui/styles.css");
    println!("cargo:rerun-if-changed=ui/index.html");
    tauri_build::build();

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        // Integration tests also need the app's Common Controls v6 manifest.
        let out_dir = std::env::var_os("OUT_DIR").expect("Cargo must provide OUT_DIR");
        // The resource filename follows the build host used by embed-resource.
        let filename = if cfg!(all(windows, not(target_env = "msvc"))) {
            "libresource.a"
        } else {
            "resource.lib"
        };
        let resource = std::path::Path::new(&out_dir).join(filename);
        println!("cargo:rustc-link-arg-tests={}", resource.display());
    }
}
