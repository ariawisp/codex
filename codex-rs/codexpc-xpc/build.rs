fn main() {
    #[cfg(target_os = "macos")]
    {
        let mut build = cc::Build::new();
        build.file("src/codexpc_xpc.m");
        // Compile without ARC because XPC's xpc_release macro expands to
        // an explicit Objective-C release, which is disallowed under ARC.
        build.flag("-Wno-deprecated-declarations");
        build.compile("codexpc_xpc");

        // Ensure frameworks resolve even when cargo invokes clang without a sysroot.
        // Prefer Xcode SDK path via xcrun; fall back to SDKROOT if set.
        let mut sdk_path: Option<String> = None;
        if let Ok(out) = std::process::Command::new("xcrun")
            .args(["--show-sdk-path"])
            .output()
        {
            if out.status.success() {
                let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !s.is_empty() { sdk_path = Some(s); }
            }
        }
        if sdk_path.is_none() {
            if let Ok(s) = std::env::var("SDKROOT") { if !s.trim().is_empty() { sdk_path = Some(s); } }
        }
        if let Some(sdk) = sdk_path {
            println!("cargo:rustc-link-search=framework={}/System/Library/Frameworks", sdk);
        }

        // macOS SDKs ship XPC inside libSystem; no explicit xpc link required.
        println!("cargo:rustc-link-lib=framework=Foundation");
        println!("cargo:rustc-link-lib=framework=CoreFoundation");
    }
}
