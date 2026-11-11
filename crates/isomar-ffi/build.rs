//! On a phone or tablet the C libraries are static (scripts/deps/build-mobile-deps.sh), and the pkg-config
//! file glib installs as a meson subproject leaves libffi out of gobject's static link line: it is added
//! here from the same prefix pkg-config reads (`PKG_CONFIG_LIBDIR`, `<prefix>/lib/pkgconfig`). On Android,
//! libffi also needs `__clear_cache` from clang's compiler-rt builtins, which rustc does not link (it links
//! with -nodefaultlibs): the NDK's clang says where they are.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=PKG_CONFIG_LIBDIR");
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if os != "ios" && os != "android" {
        return;
    }
    let Some(pc) = std::env::var_os("PKG_CONFIG_LIBDIR") else { return };
    let Some(lib) = std::path::Path::new(&pc).parent() else { return };
    if lib.join("libffi.a").is_file() {
        println!("cargo:rustc-link-search=native={}", lib.display());
        println!("cargo:rustc-link-lib=static=ffi");
    }
    if os == "android" {
        // A link argument from this crate would not reach the final library (it is a dependency of
        // isomar-android); a static library does.
        let target = std::env::var("TARGET").unwrap_or_default();
        let cc = [format!("CC_{}", target.replace('-', "_")), format!("CC_{target}"), "CLANG_PATH".into(), "CC".into()]
            .into_iter()
            .find_map(|v| std::env::var(v).ok());
        if let Some(cc) = cc
            && let Ok(out) = std::process::Command::new(cc).args([&format!("--target={target}"), "-rtlib=compiler-rt", "--print-libgcc-file-name"]).output()
        {
            let path = std::path::PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
            if let (Some(dir), Some(name)) = (path.parent(), path.file_stem().and_then(|n| n.to_str()))
                && path.is_file()
            {
                println!("cargo:rustc-link-search=native={}", dir.display());
                println!("cargo:rustc-link-lib=static={}", name.trim_start_matches("lib"));
            }
        }
    }
}
