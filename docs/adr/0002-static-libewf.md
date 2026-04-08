# ADR 0002 — libewf Integration: Vendored Autotools Build

**Status:** Accepted

## Context

iridium must produce forensically correct EWF images. libewf is the authoritative
implementation of the Expert Witness Format used by EnCase, X-Ways, and other tools.
Options considered: subprocess (ewfacquire), dynamic linking, static linking (system
package), static linking (vendored source).

## Decision

Vendor libewf as a git submodule (`vendor/libewf`, pinned to tag `20240506`) and
compile it statically via `build.rs` using the autotools chain:

1. `synclibs.sh` — clones the required libyal sub-libraries (libcerror, libcdata, …)
   from GitHub into the build source tree.
2. `autogen.sh` — generates the `configure` script.
3. `./configure --enable-static --disable-shared --without-openssl --with-zlib`
   (out-of-tree, under `$OUT_DIR/libewf-build/`).
4. `make && make install` into `$OUT_DIR/libewf-install/`.

Autotools (`autoconf`, `automake`, `libtool`, `gettext`), `pkg-config`, `git`, and
`zlib` development headers are **hard build-time system requirements**.

A `system-libewf` Cargo feature allows developers to use a system-installed libewf
via pkg-config instead of the vendored build (convenience only, not for releases).

## Consequences

- Single binary guarantee is maintained.
- Reproducible builds: the exact libewf revision is pinned by the submodule
  commit recorded in the superproject (tag `20240506` documented above).
  `.gitmodules` only records the URL and path, not the pinned commit.
- Build hosts must have autotools installed for vendored builds; CI uses
  `libewf-dev` + `--features system-libewf` for speed (vendored build is
  wired up in Phase 8 for release artifacts).
- The sub-libraries needed by libewf are fetched from GitHub at first build;
  subsequent builds reuse the compiled `.a` cached in `$OUT_DIR`.
- Cross-compilation for musl is satisfied via `LIBEWF_STATIC_DIR=<dir>` pointing
  to a pre-compiled musl-linked `libewf.a` (wired up in Phase 8).
