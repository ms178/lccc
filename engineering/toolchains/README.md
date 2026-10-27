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

## Freshness (2026-10-27 audit)

Every package carried by the transport tags, this lock and the
digest-pinned installers was audited against its latest public release
on 2026-10-27 (`engineering/FOLLOWUP-2026-10-27-tag-hash-cache-refresh.md`).
Rust 1.99.1 / Clang 23.1.4 above, GCC 16.2.0, binutils 2.47, Arm A64 ISA
XML 2026-09, ICX 2026.2, Valgrind 3.28.0 and the bookworm .deb snapshot
were already current. Three payloads were stale and are now restored
from digest-pinned upstream sources rather than the old tag bytes, so
the old copies are updated correctly instead of being relabelled:

* **mold 2.42.1 → 3.0.0** (`scripts/ci_install_mold_oracle.sh`; the
  source-built linker oracles use the mold-cli Cargo features
  `x86_64,i386` — mold 3.x removed CMake/`-DMOLD_TARGETS=`),
* **QEMU user 8.2.2 → 11.1.2** (`scripts/ensure_qemu_user.sh`),
* **kernel `relocs` v6.12 → v7.3** (`tests/linker/setup_kernel_tools.sh`,
  whose fast path now verifies a build stamp, so a stale tool in a
  warm prefix is rebuilt rather than reused).

Re-run the audit any time with
`python3 scripts/check_toolchain_cache_freshness.py` (online) or
`--offline` for the cross-file consistency half that the unit tests
enforce. The 1.97.0 `rust-cache-*` tag stays behind on purpose: it is
the MSRV floor recorded in `Cargo.toml`, not a stale current toolchain.

## Network limitation

Downloading release assets redirects to `release-assets.githubusercontent.com`.
The current Arena sandbox allowlist blocks that host, so the published payloads
cannot yet be restored locally here. Publication/version guards ran on GitHub;
that is not a claim of a successful local Rust/Clang installation. An environment
that permits the official GitHub asset redirect can restore this same lock.

## GitHub-only agent cache

Arena-style agents often cannot reach the release-asset CDN, Debian mirrors,
kernel.org or Arm's CDN.  `scripts/publish_agent_toolchain_cache.sh` therefore
publishes an immutable orphan tag (default `harness-cache-6`) whose tree holds
every current toolchain as `<name>.partNN` blobs.  The consumer,

    python3 scripts/restore_agent_toolchain_cache.py --tag harness-cache-6 --prefix /path

fetches that one ref from `origin`, reassembles the parts, verifies
`MANIFEST.sha256` and the reviewed lock in `engineering/toolchains/`, extracts
the toolchains, and writes `env.sh`.  The tag carries GCC 16.2.0, Clang 23.1.4,
Rust 1.99.1, the 1.97.0 MSRV floor, mold 3.0.0, bookworm-compatible binutils 2.47 binaries built once from pinned sources, QEMU user 11.1.2, kernel relocs v7.3, Arm A64 ISA XML 2026-09,
the bookworm runtime debs, and the binutils 2.47 source tarball.  Publishing is
opt-in via the `Publish GitHub-only agent toolchain cache` workflow; no ordinary
PR check downloads these bytes.
