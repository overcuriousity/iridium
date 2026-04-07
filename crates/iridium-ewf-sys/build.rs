// iridium-ewf-sys build script
//
// Linking strategy (in priority order):
//
//   1. LIBEWF_STATIC_DIR=<dir>  — use a pre-built static libewf.a in <dir>.
//      Intended for cross-compilation (e.g. musl target in CI).
//      The caller is responsible for also providing compatible zlib / openssl / uuid.
//
//   2. pkg-config               — find a system (or sysroot) install of libewf.
//      Tries static linking first; falls back to dynamic if static .pc is absent.
//
//   3. Graceful degradation      — emit a cargo:warning and continue.
//      `cargo check` / `cargo clippy` succeed; `cargo build` will fail at
//      link time with a clear missing-symbol error, which is expected on
//      development machines without libewf installed.
//
// Vendored autotools build (build libewf from vendor/libewf via ./autogen.sh +
// ./configure) will be added in Phase 1.5 once CI cross-compile toolchains are
// fully locked down (see docs/adr/0002-static-libewf.md).

use std::env;

fn main() {
    println!("cargo:rerun-if-env-changed=LIBEWF_STATIC_DIR");
    println!("cargo:rerun-if-env-changed=LIBEWF_NO_PKG_CONFIG");

    // Strategy 1: caller provides a directory with libewf.a already compiled
    if let Ok(dir) = env::var("LIBEWF_STATIC_DIR") {
        println!("cargo:rustc-link-search=native={dir}");
        println!("cargo:rustc-link-lib=static=ewf");
        // libewf typically needs these; adjust if your build differs.
        println!("cargo:rustc-link-lib=static=z");
        println!("cargo:rustc-link-lib=static=crypto");
        println!("cargo:rustc-link-lib=uuid");
        return;
    }

    // Strategy 2: pkg-config
    if env::var("LIBEWF_NO_PKG_CONFIG").is_err() {
        // Try static first
        if pkg_config::Config::new().statik(true).probe("libewf").is_ok() {
            return;
        }
        // Fall back to dynamic (acceptable for dev builds)
        if pkg_config::Config::new().probe("libewf").is_ok() {
            return;
        }
    }

    // Strategy 3: graceful degradation — check / clippy still work.
    println!(
        "cargo:warning=iridium-ewf-sys: libewf not found. \
         `cargo build` will fail at link time. \
         Options: \
         (a) install libewf-dev and ensure pkg-config can find it, \
         (b) set LIBEWF_STATIC_DIR=/path/to/dir-containing-libewf.a, \
         (c) see docs/adr/0002-static-libewf.md for the vendored build."
    );
}
