fn main() {
    println!("cargo:rerun-if-changed=src/containers/apple/xpc.c");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        cc::Build::new()
            .file("src/containers/apple/xpc.c")
            .flag("-fblocks")
            .flag("-std=c11")
            .warnings_into_errors(true)
            .compile("reswho_apple_xpc");
    }
}
