# Reviewed toolchain cache

`cache-lock.json` pins Rust **1.99.1** (rustc, cargo, rustfmt, clippy and
standard library) and Clang **23.1.4** (driver, runtime dependencies and resource
headers). The exact-version hosted publisher completed successfully in the run
and job recorded in the lock. Clang's upstream archive digest is independently
pinned by `scripts/ci_install_clang_oracle.sh`. Rust is installed through the
exact-version official rustup action before packaging.

The payloads are content-addressed assets on the **existing** `harness-cache-4`
release. No transport tag is created or moved; old source-tag payloads remain
unchanged. In particular, the historical tag manifest describes **23.1.3** and
must not be presented as a 23.1.4 digest. Large archives are never committed to
the compiler branch. Asset SHA-256 values were cross-checked with GitHub's
server-side digests after the publisher's exact-version guards succeeded.

Restore with:

```sh
python3 scripts/restore_toolchain_cache.py --prefix /path/to/oracles
```

The legacy combined installer delegates Rust/Clang restoration to this lock
while retaining its GCC 16.2.0, mold and binutils transport handling. Restore
verifies the pinned hash **before** extraction, uses Python's secure tar data
filter, probes the binaries before atomically installing, and refuses unrelated
pre-existing directories. Unpatched Python versions without that filter need
`backports.tarfile`; unrestricted extraction is never a fallback.

The repository still tracks Rust `stable`; the deliberate Rust 1.97 MSRV lane
remains a compatibility check, not the normal compiler toolchain. Historical
red-team versioned invocations now use 1.99.1.

Refresh through the explicit `refresh_toolchain_cache` input on the Research
workflow, then review and commit the resulting lock. Publishing never overwrites
an asset or changes a Git ref. Ordinary Research acceptance remains unconditional.

## Network limitation

Downloading release assets redirects to `release-assets.githubusercontent.com`.
The current Arena sandbox allowlist blocks that host, so the published payloads
cannot yet be restored locally here. Publication/version guards ran on GitHub;
that is not a claim of a successful local Rust/Clang installation. An environment
that permits the official GitHub asset redirect can restore this same lock.
