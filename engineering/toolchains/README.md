# Reviewed toolchain cache

Two locks describe two transports, and both now carry **only current tools**:

* `cache-lock.json` pins Rust **1.99.1** (rustc, cargo, rustfmt, clippy and the
  standard library) on the `harness-cache-4` release. Rust is installed through
  the exact-version official rustup action before packaging.
* `agent-cache-lock.json` pins the immutable `harness-cache-8` tag, its
  tag/commit/tree object IDs, the `CACHE.json` SHA-256, and the SHA-256 of
  every payload in that tag.

Clang is pinned to **23.1.5** by `scripts/ci_install_clang_oracle.sh`, which
verifies the official LLVM release asset against the SHA-256 GitHub publishes
for it and refuses to exit 0 unless `clang -dumpversion` prints 23.1.5. That
installer is the single live Clang pin *and* the single source of the cache
payload: `scripts/publish_agent_toolchain_cache.sh` derives the version from
it, stages its verified prefix, and hard-fails on any retired version. The
obsolete 23.1.4 release asset and the old `harness-cache-7` tag were deleted.

Restore with:

```sh
python3 scripts/restore_toolchain_cache.py --prefix /path/to/oracles   # Rust
python3 scripts/restore_agent_toolchain_cache.py --prefix /path        # everything
bash scripts/ci_install_clang_oracle.sh --prefix /path/to/oracles/clang-23.1.5
```

Restoration verifies each pinned hash **before** extraction, uses Python's
secure tar data filter, probes the binaries before atomically installing, and
refuses unrelated pre-existing directories. Unpatched Python releases without
that filter need `backports.tarfile`; unrestricted extraction is never a
fallback. `env.sh` exports `CC`, `CXX`, `CLANG`, `GDB`, `RUSTC`, `CARGO`,
`MOLD`, `PATH`, `LD_LIBRARY_PATH` and `LCCC_QEMU_PREFIX`, so the DWARF gates
resolve the exact pinned debugger instead of opportunistically using PATH.

The repository tracks Rust `stable` with **no declared floor**: the 1.97.0
`rust-version` key and its hosted `msrv` job were retired on 2026-11-05 so new
language features are usable as they land. Historical red-team versioned
invocations use 1.99.1.

## Freshness (audits: 2026-10-27, 2026-11-05)

Every package carried by the tag and the digest-pinned installers was audited
against its latest public release on 2026-10-27
(`engineering/FOLLOWUP-2026-10-27-tag-hash-cache-refresh.md`) and again on
2026-11-05 (`engineering/FOLLOWUP-2026-11-05-clang-2315-gdb-181.md`).
Current pins: Rust 1.99.1, Clang 23.1.5, GCC 16.2.0, **GDB 18.1**
(`-march=haswell`), binutils 2.47, mold 3.0.0, QEMU user 11.1.3, kernel
`relocs` v7.3.1, Arm A64 ISA XML 2026-09, ICX 2026.2, Valgrind 3.28.0 and
the bookworm .deb snapshot.

`harness-cache-8` is published from those pins. It no longer contains the
retired Rust 1.97.0 MSRV payload, Clang 23.1.4, QEMU 11.1.2 or relocs v7.3,
and it adds the prebuilt GDB 18.1 payload that the DWARF gates require. The
publisher enforces this: a content guard rejects any payload whose name
carries a retired version, and the new regression suite executes that guard
against clean and obsolete fixtures. `scripts/check_toolchain_cache_freshness.py`
compares every lock entry against its live enforcing pin — there are **no
superseded-payload exceptions left to bless**.

Re-run the audit any time with
`python3 scripts/check_toolchain_cache_freshness.py` (online) or `--offline`
for the cross-file consistency half that the unit tests enforce.

## GitHub-only agent cache

Arena-style agents often cannot reach the release-asset CDN, Debian mirrors,
kernel.org or Arm's CDN. `scripts/publish_agent_toolchain_cache.sh` therefore
publishes an immutable orphan tag (default `harness-cache-8`) whose tree holds
every current toolchain as `<name>.partNN` blobs; each part stays below the
90 MiB Git blob limit, and `MANIFEST.sha256` covers the reassembled archives.
The consumer,

    python3 scripts/restore_agent_toolchain_cache.py --prefix /path

fetches that one ref from `origin`, reassembles the parts, verifies the
manifest and the reviewed lock, extracts the toolchains, and writes `env.sh`
plus priority-deduplicated shims. The tag carries GCC 16.2.0, Clang 23.1.5,
GDB 18.1, Rust 1.99.1, mold 3.0.0, bookworm-compatible binutils 2.47 binaries
built once from pinned sources in a Debian bookworm container, QEMU user
11.1.3, kernel relocs v7.3.1, Arm A64 ISA XML 2026-09, the bookworm runtime
debs, and the binutils 2.47 source tarball. GDB is built inside that same
bookworm container by `scripts/ci_install_gdb_oracle.sh`, so the cached binary
satisfies the same digest, `-march=haswell`, ISA-census and line-table contract
as a locally built oracle.

Publishing is opt-in via the `Publish GitHub-only agent toolchain cache`
workflow; no ordinary PR check downloads these bytes. The workflow's ERR trap
emits a GitHub annotation naming the failing line and command, because raw job
logs live on a host a reviewer cannot reach. Publishing never overwrites an
existing tag: each refresh gets a fresh name, and the obsolete tag plus any
superseded release asset are deleted after the reviewed lock switches over.

`scripts/ensure_binutils_247_binary.sh` restores just the binutils 2.47 program
set from that lock, so fresh CI runners do not rebuild binutils from source.
