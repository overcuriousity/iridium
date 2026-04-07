// iridium-ewf-sys build script
//
// Builds libewf (and its libyal sub-libraries) from the vendored source tree
// at vendor/libewf using the standard autotools chain:
//
//   1. synclibs.sh   — clones required libyal sub-libraries into a stable
//                      $OUT_DIR/libewf-deps/ directory (skipped if done)
//   2. autogen.sh    — generates the configure script
//   3. configure     — out-of-tree configure in $OUT_DIR/libewf-build/
//   4. make / make install
//
// Hard system requirements (must be present on the build host):
//   - git
//   - autoconf, automake, libtool, gettext (autopoint), pkg-config
//   - zlib development headers  (zlib1g-dev / zlib-devel)
//
// Escape hatches:
//   - LIBEWF_STATIC_DIR=<dir>  use a pre-built static libewf.a (for cross-compilation)
//   - feature "system-libewf"   use pkg-config instead (developer convenience)

use std::{
    env,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let vendor_src = manifest_dir
        .join("../../vendor/libewf")
        .canonicalize()
        .unwrap_or_else(|_| panic!(
            "vendor/libewf not found — run: git submodule update --init vendor/libewf"
        ));

    println!(
        "cargo:rerun-if-changed={}",
        vendor_src.join("configure.ac").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        vendor_src.join("synclibs.sh").display()
    );
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=LIBEWF_STATIC_DIR");

    // ── Escape hatch 1: pre-built static lib directory ───────────────────────
    if let Ok(dir) = env::var("LIBEWF_STATIC_DIR") {
        println!("cargo:rustc-link-search=native={dir}");
        println!("cargo:rustc-link-lib=static=ewf");
        emit_system_deps();
        return;
    }

    // ── Escape hatch 2: system-libewf feature → pkg-config ──────────────────
    if env::var("CARGO_FEATURE_SYSTEM_LIBEWF").is_ok() {
        if pkg_config::Config::new().statik(true).probe("libewf").is_ok() {
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

    // ── Default: vendored autotools build ────────────────────────────────────
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    // Stable directory for synced sub-libraries — persists across failed builds.
    let deps_dir = out_dir.join("libewf-deps");
    // Fresh source tree assembled for each build attempt.
    let src_dir = out_dir.join("libewf-src");
    let build_dir = out_dir.join("libewf-build");
    let install_dir = out_dir.join("libewf-install");
    let lib_file = install_dir.join("lib").join("libewf.a");

    if lib_file.exists() {
        // Already built in this OUT_DIR — skip everything.
        println!(
            "cargo:rustc-link-search=native={}",
            install_dir.join("lib").display()
        );
        println!("cargo:rustc-link-lib=static=ewf");
        emit_system_deps();
        return;
    }

    // 1. Sync libyal sub-libraries into the stable deps dir (once per OUT_DIR).
    //    synclibs.sh clones from GitHub; guard with a sentinel file.
    let synced_sentinel = deps_dir.join(".synced");
    if !synced_sentinel.exists() {
        // Copy synclibs.sh into a scratch dir so we can run it without touching
        // the read-only vendor submodule.
        fs::create_dir_all(&deps_dir).expect("create deps dir");
        let scratch = out_dir.join("synclibs-scratch");
        fs::create_dir_all(&scratch).expect("create scratch dir");
        fs::copy(vendor_src.join("synclibs.sh"), scratch.join("synclibs.sh"))
            .expect("copy synclibs.sh");

        // synclibs.sh clones each lib into the *current* directory, so run it
        // from deps_dir.
        run(Command::new("sh")
            .arg(scratch.join("synclibs.sh").canonicalize().unwrap())
            .current_dir(&deps_dir));

        fs::write(&synced_sentinel, "").expect("write sentinel");
    }

    // 2. Assemble src_dir: vendor source + synced sub-libraries.
    copy_dir_all(&vendor_src, &src_dir)
        .unwrap_or_else(|e| panic!("Failed to copy vendor/libewf: {e}"));
    // Copy each sub-library into the source tree.
    for entry in fs::read_dir(&deps_dir).expect("read deps_dir") {
        let entry = entry.expect("read entry");
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().unwrap().to_str().unwrap();
            let dest = src_dir.join(name);
            if !dest.exists() {
                copy_dir_all(&path, &dest).unwrap_or_else(|e| {
                    panic!("Failed to copy sub-lib {name}: {e}")
                });
            }
        }
    }

    // 3. Generate configure script.
    run(Command::new("sh").arg("autogen.sh").current_dir(&src_dir));

    // 4. Out-of-tree configure.
    fs::create_dir_all(&build_dir).expect("create build dir");
    fs::create_dir_all(&install_dir).expect("create install dir");
    run(Command::new(src_dir.join("configure"))
        .current_dir(&build_dir)
        .arg(format!("--prefix={}", install_dir.display()))
        .arg("--enable-static")
        .arg("--disable-shared")
        .arg("--without-openssl") // use libewf's internal hash implementations
        .arg("--with-zlib")
        .arg("--disable-python")
        .arg("--quiet"));

    // 5. Build and install.
    let jobs = available_parallelism();
    run(Command::new("make")
        .arg(format!("-j{jobs}"))
        .current_dir(&build_dir));
    run(Command::new("make").arg("install").current_dir(&build_dir));

    // 6. Emit link directives.
    println!(
        "cargo:rustc-link-search=native={}",
        install_dir.join("lib").display()
    );
    println!("cargo:rustc-link-lib=static=ewf");
    emit_system_deps();
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn emit_system_deps() {
    println!("cargo:rustc-link-lib=z"); // zlib (DEFLATE compression)
}

fn run(cmd: &mut Command) {
    let display = format!("{cmd:?}");
    let status = cmd
        .status()
        .unwrap_or_else(|e| panic!("Failed to start `{display}`: {e}"));
    if !status.success() {
        panic!("`{display}` exited with {status}");
    }
}

fn available_parallelism() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2)
}

fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    if dst.exists() {
        fs::remove_dir_all(dst)?;
    }
    let status = Command::new("cp")
        .args(["-a", src.to_str().unwrap(), dst.to_str().unwrap()])
        .status()?;
    if !status.success() {
        return Err(std::io::Error::other(format!(
            "cp -a {src:?} {dst:?} failed"
        )));
    }
    Ok(())
}
