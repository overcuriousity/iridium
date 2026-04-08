// iridium-ewf-sys build script — Phase 1
//
// Resolution order:
//   1. LIBEWF_STATIC_DIR=<dir>   pre-built libewf.a (cross-compilation / CI)
//   2. feature "system-libewf"   pkg-config, required (panics if not found)
//   3. opportunistic pkg-config  silent probe, no feature flag needed
//   4. graceful warning          emit cargo:warning; iridium-ewf-sys will not
//                                link until one of the above succeeds.
//
// The full vendored autotools build (synclibs.sh + autoconf chain) is
// deferred to Phase 8 (Hardening) per docs/adr/0002-static-libewf.md.

use std::{env, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=LIBEWF_STATIC_DIR");

    // ── 1. Pre-built static lib directory ────────────────────────────────────
    if let Ok(dir) = env::var("LIBEWF_STATIC_DIR") {
        println!("cargo:rustc-link-search=native={dir}");
        println!("cargo:rustc-link-lib=static=ewf");
        println!("cargo:rustc-link-lib=z");
        return;
    }

    // ── 2. system-libewf feature → pkg-config (required) ─────────────────────
    if env::var("CARGO_FEATURE_SYSTEM_LIBEWF").is_ok() {
        if pkg_config::Config::new()
            .statik(true)
            .probe("libewf")
            .is_ok()
        {
            return;
        }
        if pkg_config::Config::new().probe("libewf").is_ok() {
            return;
        }
        panic!(
            "Feature `system-libewf` is set but libewf was not found via pkg-config.\n\
             Install libewf-dev (Debian/Ubuntu) or libewf-devel (Fedora/RHEL)."
        );
    }

    // ── 3. Opportunistic pkg-config probe ────────────────────────────────────
    // Lets `cargo check --workspace` succeed on machines that already have
    // libewf installed, without requiring the full autotools chain.
    if pkg_config::Config::new()
        .statik(true)
        .probe("libewf")
        .is_ok()
    {
        return;
    }
    if pkg_config::Config::new().probe("libewf").is_ok() {
        return;
    }

    // ── 4. Graceful warning ───────────────────────────────────────────────────
    // libewf was not found.  The vendored autotools build is implemented in
    // Phase 8.  For now, `cargo check` succeeds; linking iridium-ewf-sys
    // requires one of the escape hatches above.
    println!(
        "cargo:warning=libewf not found: iridium-ewf-sys will not link. \
         Set LIBEWF_STATIC_DIR, enable the `system-libewf` feature, or \
         install libewf-dev. (Vendored build: Phase 8)"
    );
}

/// Returns the HEAD commit hash of a git repository, or "unknown".
/// Used by the (Phase 8) vendored build to validate the synclibs sentinel.
#[allow(dead_code)]
fn git_head_rev(repo_path: &std::path::Path) -> String {
    Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo_path)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_owned())
        .unwrap_or_else(|| "unknown".to_owned())
}
