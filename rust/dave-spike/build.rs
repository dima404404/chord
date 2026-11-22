use std::env;

fn main() {
    println!("cargo:rerun-if-changed=cpp/dave.h");
    println!("cargo:rerun-if-changed=cpp/dave_c_api.cpp");

    let crypto_includes = if env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        // Keep the matching DLLs in Cargo's runtime search path for test binaries.
        vcpkg::Config::new()
            .find_package("openssl")
            .expect("install OpenSSL for the MSVC target triplet with vcpkg")
            .include_paths
    } else {
        pkg_config::Config::new()
            .probe("libcrypto")
            .expect("system OpenSSL libcrypto must be installed for the DAVE C++ backend")
            .include_paths
    };

    cc::Build::new()
        .cpp(true)
        .file("cpp/dave_c_api.cpp")
        .include("cpp")
        .includes(crypto_includes)
        .std("c++17")
        .opt_level(2)
        .warnings(false)
        .compile("dave_c_spike");
}
