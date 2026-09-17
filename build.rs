fn main() {
    // Record the triple this binary was built for so `--update` can pick the
    // matching release asset at runtime.
    let target = std::env::var("TARGET").unwrap_or_else(|_| "unknown".into());
    println!("cargo:rustc-env=BUILD_TARGET={target}");
    println!("cargo:rerun-if-changed=build.rs");
}
