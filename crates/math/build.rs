//! With `--features exprtk` this compiles the vendored ExprTk behind a tiny C shim, for the oracle tests only.
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=oracle/exprtk_shim.cpp");
    if std::env::var_os("CARGO_FEATURE_EXPRTK").is_some() {
        cc::Build::new()
            .cpp(true)
            .std("c++17")
            .opt_level(1)
            .warnings(false)
            .include("../../third_party/exprtk")
            .file("oracle/exprtk_shim.cpp")
            .compile("exprtk_shim");
        println!("cargo:rustc-link-lib=stdc++");
    }
}
