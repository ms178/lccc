# FOLLOWUP 2026-10-27 — Tag hash cache: every package audited against its latest public release

Task: *“Update the packages in the tag hash cache to their latest
publicly available release versions. Make sure the old ones get updated
correctly.”*

Base: `main` @ `b61ad75f2e1e8bf1b0d8447904dbfdf39a767f7c` (2026-10-27,
#986; this patch was rebased onto it — zero file overlap — and revalidated
with a full `ci_local.sh --fast` run on the rebased tree).
Audit date: **2026-10-27**. Host: 2 vCPU / 1.9 GiB RAM + 8 GiB swap (two swap files)
(active), Debian 13, lccc built with the `fastbuild` preset
(`-O1` opt-level, `-j2`), Rust/Cargo/rustfmt/Clippy **1.99.1**.

**Reproduce the entire audit:** `python3 scripts/check_toolchain_cache_freshness.py`
(exit 0 = every pin is the latest public release; `--offline` runs the
cross-file consistency half without network and is what the unit tests enforce).

---

## 1. Inventory — what the “tag hash cache” actually is

| Transport | Payloads |
|---|---|
| tag `harness-cache-1` | `gcc-16-bookworm-x86_64.tar.xz`, `binutils-2.47.tar.xz`, `mold-2.42.1-x86_64`, `debs-bookworm.tar.xz` |
| tag `harness-cache-2` | `ISA_A64_xml_A_profile-2026-09_md.tar.gz`, `kernel-tools/` (Linux `relocs` sources) |
| tag `harness-cache-3` | `qemu-{aarch64,arm,i386,riscv64}.xz` |
| tag `harness-cache-4` (tree) | `clang-23.1.3-linux-x64.tar.xz` |
| release `harness-cache-4` (content-addressed assets, `cache-lock.json`) | `clang-23.1.4-<sha>.tar.xz`, `rust-1.99.1-<sha>.tar.xz` |
| tag `rust-cache-1.97.0` | rustc/cargo/rust-std/clippy/rustfmt 1.97.0 |

Each git tag carries a `MANIFEST.sha256`; the release assets carry
GitHub server-side digests. **A `main`-branch patch cannot rewrite the
binary blobs inside orphan tags** (and must never *relabel* them — see
`engineering/toolchains/README.md`). The correct update mechanism —
the one this session used for every stale payload — is therefore:

> point each consumer at a **digest-pinned upstream provisioner** for
> the new version (verifying bytes before use, refusing/replacing a
> stale install), keep tag transport only where no upstream binary
> exists (the custom bookworm GCC 16.2 prefix), and leave the hosted
> `publish_toolchain_cache.py` flow to re-issue content-addressed
> assets for Rust/Clang when those next move.

## 2. Verdicts (2026-10-27, evidence in §5)

| Package | Was | Latest public | Verdict |
|---|---|---|---|
| GCC | 16.2.0 | 16.2.0 (ftp.gnu.org, no 16.3) | ✅ current |
| binutils / GAS / bfd | 2.47 | 2.47 | ✅ current |
| **mold** | 2.42.1 (2026-09-11) | **3.0.0 (2026-10-05)** | 🔴 **updated → 3.0.0** |
| Clang (cache-lock asset) | 23.1.4 | 23.1.4 (2026-10-20) | ✅ current |
| Clang (tag tree) | 23.1.3 | 23.1.4 | ⚪ historical; already superseded by the asset, consumers guarded |
| Rust (cache-lock asset) | 1.99.1 | 1.99.1 (2026-10-16) | ✅ current |
| Rust (`rust-cache-1.97.0`) | 1.97.0 | — | ⚪ **intentional MSRV floor** (`Cargo.toml`), not staleness |
| **QEMU user** | 8.2.2 (Ubuntu, 2024) | **11.1.2** | 🔴 **updated → 11.1.2** |
| **kernel `relocs`** | v6.12 | **v7.3** (mainline 2026-10-18) | 🔴 **updated → v7.3** |
| Arm A64 ISA XML | 2026-09 | 2026-09 (2026-10/11/12 permalinks 404) | ✅ current |
| debs-bookworm | libc `deb12u14`, gcc-12 `deb12u1`, valgrind 3.19.0-1, pyyaml 6.0-3+b2 | identical in bookworm (madison) | ✅ current *for bookworm* |
| ICX (adjacent pin) | 2026.2 | 2026.2 (Intel APT index max) | ✅ current |
| Valgrind (adjacent pin) | 3.28.0 | 3.28.0 | ✅ current |
| lld (adjacent pin) | 23.1.x | 23.1.4 (LLVM line) | ✅ current line |

## 3. What changed (all in `ms178-1.patch`, snapshots S01–S05)

### mold 2.42.1 → 3.0.0 — including the build-system break
mold 3.0 is the Rust rewrite (“drop-in replacement for 2.42.1”, upstream
release notes). It **removed CMake and `-DMOLD_TARGETS=`**, so bumping
only the version string would have broken both source-built linker
oracles. Changes:

* `scripts/ci_install_mold_oracle.sh` *(new)* — pins the official
  `mold-3.0.0-x86_64-linux.tar.gz` by its GitHub-published SHA-256
  (`6c90d4a4…21189`, re-verified live), refuses foreign/stale prefixes,
  exit 0 requires a real link probe (`-fuse-ld=mold`, probe exits 42).
* `scripts/oracle_toolchains_from_tags.sh` — `MOLD_VERSION=3.0.0`, mold
  delegated to the new installer (`$PREFIX/mold-3.0.0`, symlinked into
  `$PREFIX/bin`); the 2.42.1 tag member is no longer materialised.
  Offline verification now probes 3.0.0.
* `scripts/ci_prepare_research_tools.sh` — current-linker bootstrap is
  mold 3.0.0 (no more `git show harness-cache-1:mold-2.42.1…`).
* `scripts/ci_install_valgrind_current.sh` — requires `mold 3.0.0`.
* `tools/linker/setup_oracles.sh` + `tests/linker/setup_oracles.sh`
  (the linker oracles, per the owner's build/version preferences) —
  mold **v3.0.0** from source with the x86-only preset in its new
  spelling:
  `cargo build --release -p mold-cli --no-default-features --features x86_64,i386`
  (`+system-allocator` in tests/linker, preserving the old
  `MOLD_USE_MIMALLOC=OFF` policy), then `install-mold.sh`.
  `tools/linker` also deletes stale versioned `mold-<old>` wrappers so
  an old oracle cannot keep vouching for itself in `$LINKDIR`.
* `tests/linker/real_workloads.py` — pinned wrapper `mold-3.0.0`;
  fixtures/mocks in `test_toolchain_cache.py`,
  `test_valgrind_provisioning.py`, `check_ci_link_oracle_provisioning.py`,
  `check_linker_oracle_versions.sh` (+ a negative case: a 3.0.0 banner
  must not satisfy the superseded 2.42.1 pin).

*Validated:* x86-only source build 1 m 38 s @ -j2, 8.6 MB binary
(all-target upstream binary: 41 MB), banner embeds the upstream
release commit `8de38c35…`; x86-64 link probe (exit 7), i386
(`-m elf_i386`) link + QEMU run OK, RISC-V emulation *rejected*
(`unsupported target: riscv64`) — proof the target restriction really
applied.

### QEMU user 8.2.2 → 11.1.2
* `scripts/ensure_qemu_user.sh` *(new)* — installs
  `qemu-{aarch64,arm,i386,riscv64}` **11.1.2** from Debian sid's
  `qemu-user` 11.1.2+ds-3 (the static-binaries package that replaced
  `qemu-user-static`; `Built-Using: gcc-16 (= 16.2.0-3)`). The `.deb`
  is pinned by the signed Packages-index SHA-256
  (`4bacc2bf…93655`); `snapshot.debian.org/file/<sha1>` is the
  content-addressed fallback for when the live pool prunes it — the
  same SHA-256 gate applies to either transport. A warm prefix is
  reused only if *every* binary reports 11.1.2 (an in-place 8.2.2 is
  replaced — tested); staging+validate+publish; `--self-test` covers
  accept-11.1.2 / reject-8.2.2 / reject-11.1.20 / reject-incomplete.
  The self-test caught two real bugs during development (a broken
  `case` pattern and a success path exiting 1) — both fixed.

### Kernel `relocs` v6.12 → v7.3
* `tests/linker/setup_kernel_tools.sh` — `PINNED_KVER=v7.3`, new
  SHA-256 pins (only `relocs.c` changed: `acbeca7b…`; cross-checked
  identical between the torvalds and stable kernel.org trees).
  v7.3's tool drops the percpu/`relocs32neg` handling and learns
  `R_X86_64_REX_GOTPCRELX`; the consumer test
  (`emit_relocs_kernel_relocs_tool`) compares the *same* tool over
  lccc vs bfd images, so it is version-agnostic — verified directly
  (see §4).
* **Defect fixed:** the old script exited 0 on *any* existing
  `$BIN/relocs`, so every warm prefix would have kept a v6.12-built
  tool forever after a pin bump. It now writes a build stamp
  (kver + pins digest), requires stamp+source verification on the
  fast path, and refetches when pre-placed sources fail the new pins
  (the exact harness-cache-2 update case). Tested: fresh, fast-path,
  stale-v6.12 rebuild, wrong `--kver` rejection.

### Freshness checker (so this cannot silently recur)
* `scripts/check_toolchain_cache_freshness.py` *(new)*
  `--offline`: reads every pinned version from the file that enforces
  it and fails on any cross-file disagreement (incl. binutils/GCC
  digests stated in multiple places, cache-lock content addressing,
  and “nothing may reference the superseded 23.1.3/2.42.1 payloads”).
  Online: compares against upstream latest (GitHub releases for
  LLVM/Rust/mold **including asset-digest cross-checks**, GNU and
  sourceware listings, kernel.org, QEMU, Intel APT index, Arm
  permalinks via curl — their CDN 403s Python TLS — and Debian
  madison for the bookworm snapshot). Intentional exceptions are
  INFO rows. One dead upstream degrades to an ERROR row (exit 2),
  never a fake green. Unit tests in `tests/corpus/test_toolchain_cache.py`
  (discovered by the existing corpus unittest gate).
* Docs: `engineering/toolchains/README.md` (Freshness section),
  `scripts/README.md` (3 new tool rows), this file.

### Explicit non-changes
* `cache-lock.json` is **byte-identical**: 1.99.1/23.1.4 are the
  latest public releases and the recorded asset digests still match
  GitHub's server-side digests (re-verified 2026-10-27). Fabricating
  a new lock without a hosted publish would be a lie.
* `rust-cache-1.97.0` stays: it is the tested **MSRV floor**
  (`Cargo.toml: rust-version = "1.97.0"` + the deliberate 1.97 CI lane).
  Updating it to 1.99.1 would destroy the very check it exists for.
  (1.97.1 is the latest 1.97 patch; an MSRV floor is by definition
  the *.0* — bumping the floor is a policy decision for the owner,
  §6.)
* The 23.1.3 tag tree / 2.42.1 tag binary stay in history as
  historical transport bytes; they are unreferenced by every
  restorer (guarded by the offline freshness checks).
* CI's distro lanes (`ci_install_link_oracles.sh`,
  `ci_provision_toolchain.sh` apt extras) keep their presence-only
  contracts by design; the *exact-version* linker oracle is
  `tools/linker/setup_oracles.sh` / `tests/linker/setup_oracles.sh`.

## 4. Validation

| Check | Result |
|---|---|
| `check_toolchain_cache_freshness.py` (online) | **12/12 CURRENT**, 2 INFO, 0 stale/error |
| `… --offline` + unit tests | 16 passed (incl. 9 subtests) |
| mold 3.0.0 installer | fresh install + idempotent rerun + foreign/old prefix rejected; link probe 42 |
| mold 3.0.0 source build (x86/i386 only) | 1 m 38 s @-j2; x86-64+i386 links run; riscv64 rejected |
| `ensure_qemu_user.sh` | self-test PASS; fresh/fast-path/stale-replace/corrupt-cache-heal; i386 guest runs |
| `setup_kernel_tools.sh` | fresh v7.3 fetch+build; stamp fast path; stale v6.12 auto-rebuild; `--text` output matches v7.3 semantics |
| `test_toolchain_cache`, `test_valgrind_provisioning`, `check_ci_link_oracle_provisioning`, `check_linker_oracle_versions`, `check_script_imports` | all PASS |
| lccc build (`fastbuild`, -O1, -j2, swap active) | clean in 5 m 01 s, warnings denied |
| full oracle restore (`oracle_toolchains_from_tags.sh`) | GCC 16.2.0 (tag) + Clang 23.1.4/Rust 1.99.1 (cache-lock assets) + mold 3.0.0 (upstream); link probes gcc-16 OK, clang-23+mold OK |
| linker `emit_relocs` family with relocs **v7.3** + mold **3.0.0** | 6/6 PASS (`emit_relocs_kernel_relocs_tool`: kernel tool derives an identical set from lccc and GNU ld) |
| Valgrind 3.28.0 via `ci_install_valgrind_current.sh` (gcc-16 + GAS 2.47 + `ld`=mold 3.0.0) | built + verified (the mold 3.0.0 gate in that script exercised end-to-end) |
| `bash scripts/ci_local.sh --fast` | **451 passed, 0 failed, 7 skipped — ALL GATES GREEN** on the original base; after rebasing onto #986 (`b61ad75…`) the full run was repeated identically: **452 passed, 0 failed, 7 skipped — ALL GATES GREEN** (pass stamp written; #986 adds one gate). First run in the under-provisioned sandbox was 448/3/7: the 3 reds were `valgrind is not on PATH` (×2) and `GNU as 2.47 required (got 2.44)` — environment, not code; after provisioning GAS 2.47 + Valgrind 3.28.0 all three gates pass, then the full rerun above |

## 5. Evidence (upstream answers, 2026-10-27)

* LLVM: `llvmorg-23.1.4` 2026-10-20 is the newest non-prerelease
  (GitHub API, descending list 23.1.4 → 23.1.0 → 22.1.8…).
* Rust: `releases/latest` = **1.99.1** (2026-10-16); 1.97.1 exists
  (2026-07-16) — MSRV note above.
* mold: `releases/latest` = **v3.0.0** (2026-10-05), asset
  `mold-3.0.0-x86_64-linux.tar.gz` digest `sha256:6c90d4a4…21189`
  (= locally computed digest). Cargo features per target crate:
  `cli/Cargo.toml` @ v3.0.0.
* GCC: ftp.gnu.org/gnu/gcc/ maximum = 16.2.0. binutils maximum = 2.47.
* QEMU: download.qemu.org maximum tarball = 11.1.2; Debian sid
  `qemu-user` = 11.1.2+ds-3, SHA-256 `4bacc2bf…93655` from the signed
  Packages index (= local), SHA-1 `6067b061…56c51` (snapshot key,
  reachable 2026-10-27). Cached tag binaries self-report
  `8.2.2 (Debian 1:8.2.2+ds-0ubuntu1.18)` — executed, not assumed.
* Kernel: kernel.org `releases.json` — mainline 7.3 (2026-10-18),
  `latest_stable` 7.3, stable 7.2.13 (2026-10-26).
* Arm: `ISA_A64_…-2026-09_…` permalink HTTP 200;
  `2026-10/11/12` → 404.
* Debian madison: bookworm `libc6 2.36-9+deb12u14`,
  `gcc-12 12.2.0-14+deb12u1`, `valgrind 1:3.19.0-1`,
  `pyyaml 6.0-3` — exactly the cached .debs' versions
  (tar listing of `debs-bookworm.tar.xz`, 19 files).
* Intel APT: max `intel-oneapi-compiler-dpcpp-cpp-runtime-*` = 2026.2.
* cache-lock assets: GitHub release `harness-cache-4` asset digests
  equal the lock's sha256 values byte-for-byte.

## 6. Follow-up / owner decisions

1. **Maintainer tag action (cannot be a main-branch patch):**
   optionally re-issue `harness-cache-3`/`harness-cache-2` payloads
   from the new provisioners' outputs for fully-offline sandboxes.
   Not required for correctness: every consumer now restores the new
   versions from authenticated upstream bytes.
2. **MSRV policy:** decide whether the floor should ever move from
   1.97.0 to 1.97.1/1.99.x; the freshness checker will keep calling
   it INFO until `Cargo.toml` changes.
3. **Next upstream moves will trip the checker first** — expected
   soonest: LLVM 23.1.5 (biweekly cadence), Rust 1.100 (2026-11/12),
   kernel 7.4-rc/mainline, mold 3.0.x. The refresh procedure is:
   bump the installer pin + digest (from the upstream API named in
   the installer's header), bump consumers, run the checker, and for
   Rust/Clang use the hosted `refresh_toolchain_cache` workflow and
   commit the new lock *after* its run — never edit digests by hand.
4. **Godbolt/CE channels** (separate from the tag cache) were not
   re-pinned in this task; `scripts/godbolt.py` resolves oracles by
   pinned aliases (`cg162`, `cclang2310`, `cicxlatest`) with its own
   re-verification story. A CE-channel audit is a sensible separate
   follow-up; do not conflate a CE alias bump with cache freshness.
5. GCC/binutils remain tag-restored (no upstream binary for the
   bookworm GCC prefix). If GCC 16.3 / binutils 2.48 ship, that is a
   tag re-issue + `MANIFEST.sha256` + three digest sites
   (`oracle_toolchains_from_tags.sh`, `ci_install_gcc_oracle.sh`,
   `ensure_gas_247.sh`/`tools/linker/setup_oracles.sh`) — the offline
   freshness checks enumerate exactly those sites.
