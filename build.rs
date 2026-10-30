fn main() {
    println!("cargo:rerun-if-env-changed=DEP_CEF_DLL_WRAPPER_CEF_DIR");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        // The installed binary finds its sibling runtime; cargo run/test also
        // need the build-tree distribution on their loader search path.
        println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN/../lib/timeslay/cef");
        if let Ok(path) = std::env::var("DEP_CEF_DLL_WRAPPER_CEF_DIR") {
            println!("cargo:rustc-link-arg=-Wl,-rpath,{path}");
        }
    }
}
