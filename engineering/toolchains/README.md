# Reviewed toolchain cache

`cache-lock.json` pins Rust **1.99.1** (rustc, cargo, rustfmt, clippy and
standard library). The exact-version hosted publisher completed successfully in
the run and job recorded in the lock. Rust is installed through the
exact-version official rustup action before packaging.

Clang is pinned to **23.1.5** by `scripts/ci_install_clang_oracle.sh`, which
verifies the official LLVM release asset against the SHA-256 GitHub publishes
for it and refuses to exit 0 unless `clang -dumpversion` prints 23.1.5. That
installer is the single live Clang pin; CI, the combined provisioner and the
agent sandbox all resolve the oracle through it.

The lock still *describes* a repacked Clang **23.1.4** asset, and that entry is
superseded, not current: a main-branch patch cannot re-issue a published
release asset, so when the pin moved to 23.1.5 the asset stayed behind. Nothing
restores Clang from the lock any more (`restore_toolchain_cache.py` lists Rust
only, and `scripts/check_toolchain_cache_freshness.py` fails the build if that
changes silently). `scripts/publish_toolchain_cache.py` re-issues the asset and
rewrites this lock at 23.1.5 the next time the hosted cache job runs; the
freshness checker asserts the 23.1.4 value on purpose, so that republication
fails the audit until the superseded note is retired with it.

The payloads are content-addressed assets on the **existing** `harness-cache-4`
release. No transport tag is created or moved; old source-tag payloads remain
unchanged. In particular, the historical tag manifest describes **23.1.3** and
must not be presented as a 23.1.5 digest, and the release asset this lock
names is **23.1.4**, which is likewise not the pinned oracle. Large archives are never committed to
the compiler branch. Asset SHA-256 values were cross-checked with GitHub's
server-side digests after the publisher's exact-version guards succeeded.

Restore with:

```sh
python3 scripts/restore_toolchain_cache.py --prefix /path/to/oracles   # Rust
bash scripts/ci_install_clang_oracle.sh --prefix /path/to/oracles/clang-23.1.5
```

The legacy combined installer delegates Rust restoration to this lock and
Clang to the digest-pinned installer above, while retaining its GCC 16.2.0,
gdb, mold and binutils transport handling. Restore
verifies the pinned hash **before** extraction, uses Python's secure tar data
filter, probes the binaries before atomically installing, and refuses unrelated
pre-existing directories. Unpatched Python versions without that filter need
`backports.tarfile`; unrestricted extraction is never a fallback.

The repository tracks Rust `stable` with **no declared floor**: the 1.97.0
`rust-version` key and its hosted `msrv` job were retired on 2026-11-05 so new
language features are usable as they land (the reasoning and the guard that
keeps the floor out are in `Cargo.toml` and in
`scripts/check_toolchain_cache_freshness.py`). Historical red-team versioned
invocations use 1.99.1.

Refresh through the explicit `refresh_toolchain_cache` input on the Research
workflow, then review and commit the resulting lock. Publishing never overwrites
an asset or changes a Git ref. Ordinary Research acceptance remains unconditional.

## Freshness (audits: 2026-10-27, 2026-11-05)

Every package carried by the transport tags, this lock and the
digest-pinned installers was audited against its latest public release
on 2026-10-27 (`engineering/FOLLOWUP-2026-10-27-tag-hash-cache-refresh.md`).
Rust 1.99.1 / Clang 23.1.5 above, GCC 16.2.0, binutils 2.47, Arm A64 ISA
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

### 2026-11-05 audit

(`engineering/FOLLOWUP-2026-11-05-clang-2315-gdb-181.md`)

* **Clang 23.1.4 → 23.1.5.** Upstream released 23.1.5 on 2026-11-03. The pin
  moved in the digest-pinned installer (asset `LLVM-23.1.5-Linux-X64.tar.zst`,
  SHA-256 `c3107384…a308065`, verified by a real 1,198,896,522-byte download and
  cross-checked against GitHub's server-side digest), and with it every gate,
  workflow, lane plan and fixture that names the oracle. The two published
  transport payloads could not move (see above) and are recorded as superseded.
* **gdb 18.1 added** (`scripts/ci_install_gdb_oracle.sh`), the latest GNU
  release (2026-09-25), built from its pinned source tarball with
  **`-march=haswell`**. Until now the DWARF gates resolved gdb opportunistically
  and *skipped* their debugger cross-check when it was absent while still
  exiting 0 — a green gate that never asked a debugger anything. GNU publishes
  no checksum, so the tarball's SHA-256 (`cd9fc3fe…e2348f`, 22,158,720 bytes)
  was computed and cross-verified byte-for-byte against **both** GNU mirrors
  (`ftp.gnu.org` and `sourceware.org`).
* **Fixed 2026-11-05:** QEMU user 11.1.2 → **11.1.3** and kernel `relocs`
  v7.3 → **v7.3.1** — the two pins that were still stale after the
  2026-10-27 refresh. QEMU was re-pinned against the live Debian sid index and
  then verified by downloading all 72,595,524 bytes and checking them against
  the pinned SHA-256; the relocs bump is a label and tree change only, because
  all six source digests are byte-identical between v7.3 and v7.3.1 (verified
  over two independent transports), and `v7.3.1` exists only in the *stable*
  kernel.org tree, so `BASE` moved with it. The online audit now exits 0.
* **Retired 2026-11-05:** the 1.97.0 MSRV toolchain and its CI runner, along
  with the `rust-cache-1.97.0` transport tag and the agent cache's
  `rust-msrv` payload entry.

Re-run the audit any time with
`python3 scripts/check_toolchain_cache_freshness.py` (online) or
`--offline` for the cross-file consistency half that the unit tests
enforce. Nothing is "behind on purpose" any more: the deliberate 1.97.0 MSRV
floor is gone, and every recorded exception is an ASSERTED superseded
*published payload* (Clang 23.1.4 in `harness-cache-4`/`harness-cache-7`, QEMU
11.1.2 and relocs v7.3 in `harness-cache-7`). Asserting the value is what makes
each exception retire itself the moment the hosted publisher re-issues that
payload.

## Network reachability (measured 2026-11-05)

This section used to state that the Arena sandbox allowlist blocks
`release-assets.githubusercontent.com`. **That is no longer true and was
re-tested:** a ranged `GET` of
`github.com/llvm/llvm-project/releases/download/llvmorg-23.1.5/LLVM-23.1.5-Linux-X64.tar.zst`
returns **HTTP 206** from the sandbox, and the full 1.2 GiB asset then
downloaded, passed its SHA-256 gate and installed. `ftp.gnu.org` and
`sourceware.org` are reachable as well.

Two consequences: `scripts/arena_provision.py` now installs Clang through the
digest-pinned installer instead of the `harness-cache-7` git blob (which can
only ever serve the last *published* payload, i.e. a stale oracle), and the
git-blob transport remains what it always was — a fallback for environments
whose allowlist is narrower than this one, not the primary path.

One real constraint did surface: `/tmp` is a **992 MiB tmpfs** here, so a
1.2 GiB download dies as curl exit 23 ("write error"), which reads like a
network failure. Both installers now preflight the free space of the download
directory and name the fix (`TMPDIR=/var/tmp`).

## GitHub-only agent cache

Arena-style agents often cannot reach the release-asset CDN, Debian mirrors,
kernel.org or Arm's CDN.  `scripts/publish_agent_toolchain_cache.sh` therefore
publishes an immutable orphan tag (default `harness-cache-7`) whose tree holds
every current toolchain as `<name>.partNN` blobs.  The consumer,

    python3 scripts/restore_agent_toolchain_cache.py --tag harness-cache-7 --prefix /path

fetches that one ref from `origin`, reassembles the parts, verifies
`MANIFEST.sha256` and the reviewed lock in `engineering/toolchains/`, extracts
the toolchains, and writes `env.sh`.  The tag carries GCC 16.2.0, Clang 23.1.4 (the superseded payload described
above; the publisher now stages the pinned 23.1.5 installer prefix, so the next
run re-issues it), Rust 1.99.1 (no MSRV payload since 2026-11-05), mold 3.0.0, bookworm-compatible binutils 2.47 binaries built once from pinned sources in a Debian bookworm container, QEMU user 11.1.2 and kernel relocs v7.3 (both superseded by the tree's 11.1.3 / v7.3.1 pins; the next publish re-issues them), Arm A64 ISA XML 2026-09,
the bookworm runtime debs, and the binutils 2.47 source tarball.  Publishing is
opt-in via the `Publish GitHub-only agent toolchain cache` workflow; no ordinary
PR check downloads these bytes.

The reviewed lock `engineering/toolchains/agent-cache-lock.json` pins the
immutable tag, its tag/commit/tree object IDs, the `CACHE.json` digest and the
SHA-256 of every payload.  `scripts/ensure_binutils_247_binary.sh` restores
just the binutils 2.47 program set from that lock, so fresh CI runners do not
rebuild binutils from source.
