fn main() {
    #[cfg(target_os = "macos")]
    {
        let mut build = cc::Build::new();
        build.file("src/codexpc_xpc.m");
        build.flag("-fobjc-arc");
        build.flag("-Wno-deprecated-declarations");
        build.compile("codexpc_xpc");
        println!("cargo:rustc-link-lib=framework=XPC");
        println!("cargo:rustc-link-lib=framework=Foundation");
        println!("cargo:rustc-link-lib=framework=CoreFoundation");
    }
}

