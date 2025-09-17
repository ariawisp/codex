fn main() {
    #[cfg(target_os = "macos")]
    {
        let mut build = cc::Build::new();
        build.file("src/codexpc_xpc.m");
        // Compile without ARC because XPC's xpc_release macro expands to
        // an explicit Objective-C release, which is disallowed under ARC.
        build.flag("-Wno-deprecated-declarations");
        build.compile("codexpc_xpc");
        println!("cargo:rustc-link-lib=framework=XPC");
        println!("cargo:rustc-link-lib=framework=Foundation");
        println!("cargo:rustc-link-lib=framework=CoreFoundation");
    }
}
