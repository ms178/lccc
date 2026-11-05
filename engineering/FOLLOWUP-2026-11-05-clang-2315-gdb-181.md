# FOLLOWUP 2026-11-05 — Clang oracle 23.1.4 → 23.1.5, GDB 18.1 (`-march=haswell`), QEMU 11.1.3 / kernel relocs v7.3.1, and the Rust MSRV floor retired

Task: *“Update all oracle scripts and the release tag hash cache from Clang
23.1.4 to 23.1.5. Just smoke testing allowed, no long CI runs. … Also add the
latest GDB release (compiled with `-march=haswell`) to the tag hash cache.”*

**Round 2 (same session, same base commit).** After the Clang/GDB work was
delivered, the audit's two remaining STALE pins were fixed (QEMU 11.1.2 →
**11.1.3**, kernel `relocs` v7.3 → **v7.3.1**) and the Rust MSRV floor was
**removed outright** — the 1.97.0 toolchain *and its CI runner* — so the tree
can use new Rust language features. The online audit now exits **0**: every
pinned package is CURRENT against live upstream. §6 documents round 2; §3 and
§4 carry both rounds.

Base: `main` @ `f530c231a8622dc7a9af73dd701b8ba891c75b39` (2026-11-05 15:10
CET, merge of #1106). Re-fetched and rebased onto it at session start.
Host: 2 vCPU (Xeon @ 2.60 GHz), **1.9 GiB RAM + a 6 GiB swapfile installed
this session** (`/swapfile`, `swapon` verified), Debian 13, `/tmp` is a
**992 MiB tmpfs**, work on `/dev/vda` (25 GiB). Python 3.13.16, host gcc
14.2.0, host clang **14** (i.e. nothing on PATH is a usable oracle — every
gate here needed the pinned install).

Per instruction this session ran **smoke tests only**: no `ci_local.sh`
mirror, so snapshots are published `ci_gate=UNGATED` and are explicitly *not*
delivery candidates until a `--fast` pass is recorded. What *was* run, and its
output, is in §4 — every claim below is backed by one of those runs.

Reproduce the whole audit in one command:
`python3 scripts/check_toolchain_cache_freshness.py` (add `--offline` for the
network-free half the unit tests enforce).

---

## 1. Clang 23.1.4 → 23.1.5

Upstream released **23.1.5 on 2026-11-03** (`llvmorg-23.1.5`), so the pin was
two days stale. The pin lives in exactly one place:

| File | What it pins |
|---|---|
| `scripts/ci_install_clang_oracle.sh` | `VERSION=23.1.5` and `SHA256=c3107384835f788768f614db610e87fba7077f125ca950eb02a78bab0a308065` for `LLVM-23.1.5-Linux-X64.tar.zst` |

Everything else either names that version or resolves the oracle through that
installer. Scope of the delivered patch: **120 files — 118 existing files
modified plus two new ones** (`scripts/ci_install_gdb_oracle.sh` and this
document); the exact `+/-` line counts are recorded per snapshot in
`artifacts/SNAPSHOT_LEDGER.md` (S03: +1624/−353), and a `git diff --stat`
against the base reports one file fewer because this document is untracked
until the snapshot commits it. The 118 modifications break down as a
pin/expectation sweep over 110 files (installer, provisioner,
publishers, `ci_verify_oracles.sh`, `ci_local.sh`, the three workflows,
`ci-lane-plan.json`, `tests/regression/lib/clang_oracle.sh`, the gates that
check `clang -dumpversion`, and their mock fixtures), a second sweep over
**18 files** whose version lives inside a *regex literal* (§5.1 — the dangerous
class), and 9 files rewritten by hand because a sweep cannot express the
change (the two locks' supersession story, the restore path, the two
publishers, the arena provisioner, the freshness checker, this doc and the
toolchains README).

The digest was verified three independent ways:

1. GitHub's server-side digest for the asset (`.digest` field of the release
   API) — `sha256:c3107384…a308065`;
2. a **real 1,198,896,522-byte download** through the updated installer, which
   passed its own `sha256sum --check` gate and then extracted the C front end;
3. the freshness checker's online `clang-asset-digest` cross-check, which stays
   silent only when the pinned digest equals GitHub's for the pinned version.

Post-install identity: `clang -dumpversion` → `23.1.5`;
`clang version 23.1.5 (…llvm-project 392dfff97d29b59d77e5cac6d4b6ef2585fe7117)`.

### 1.1 Why two payloads could not move, and how that is kept honest

A **main-branch patch cannot re-issue a published release asset or a tag
blob**. Two transports therefore still carry a repacked 23.1.4:

| Transport | Payload | Status |
|---|---|---|
| release `harness-cache-4` asset (`cache-lock.json`) | `clang-23.1.4-373e20c3….tar.xz` | superseded |
| tag `harness-cache-7` blob (`agent-cache-lock.json`) | `clang-23.1.4-linux-x64.tar.xz`, 67,322,400 B | superseded |
| tag `harness-cache-4` **git tree** | `clang-23.1.3-linux-x64.tar.xz` | historical (unchanged) |

Rather than point a lock at bytes that do not exist, this patch follows the
pattern the 2026-10-27 refresh established for mold 2.42.1 → 3.0.0:

* `scripts/restore_toolchain_cache.py` now restores **Rust only**
  (`VERSIONS = {'rust': '1.99.1'}`), with the reason in its docstring;
* `scripts/oracle_toolchains_from_tags.sh` installs Clang through
  `ci_install_clang_oracle.sh --prefix "$CLANG_ROOT"` (exactly how it already
  delegates mold), and Rust via `restore_toolchain_cache.py --tool rust`;
* `scripts/arena_provision.py` installs Clang through the same installer
  instead of the `harness-cache-7` blob — the blob can only ever serve the
  last *published* payload, i.e. a silently stale oracle (§5.4 re-tested the
  reachability assumption that justified the blob in the first place);
* `scripts/publish_toolchain_cache.py` is at 23.1.5, and
  `scripts/publish_agent_toolchain_cache.sh` now **stages the installer's
  verified prefix and packs it**, reading the version out of the installer
  (`CLANG_VERSION=$(sed -n 's/^VERSION=…')`) so the publisher cannot carry a
  second copy of the pin;
* `scripts/check_toolchain_cache_freshness.py` records both stale payloads as
  **asserted facts** (`clang-cache-asset`/`clang-agent-payload` = `23.1.4`)
  reported as `INFO`, and fails the audit if anything restores Clang from
  them. Asserting the value — instead of merely skipping the files — is what
  makes the exception **self-retiring**: the day the hosted job re-issues the
  assets at 23.1.5, the offline check breaks and forces the note to be removed.
  `tests/corpus/test_toolchain_cache.py::test_a_republished_asset_forces_the_exception_to_retire`
  proves that behaviour by mutating a copy of the lock.

**Action needed from a hosted run:** the `Publish GitHub-only agent toolchain
cache` / `refresh_toolchain_cache` workflows must run once; afterwards the two
`INFO` rows and their pins should be deleted, `clang` re-added to
`restore_toolchain_cache.py::VERSIONS`, and
`test_reviewed_lock_covers_every_restorable_tool`'s `{'clang'}` exception
dropped. That test fails until it is, by design.

### 1.2 What was deliberately *not* rewritten

Dated evidence stays truthful. `engineering/evidence/**`,
`engineering/journal/**`, `engineering/FOLLOWUP-*`, `engineering/*AUDIT*`,
`engineering/llvm-missed/*.json`, `docs/*` audit reports, `backlog.md`,
`src/**` "measured on gcc-16.2 and clang-23.1.4" comments, and the two
**generated** artifacts `tests/regression/sse2_codegen_matrix.json` (embeds
the compiler banner that produced it) and
`tests/regression/ext_vector_abi/grid_expectations.txt` (records which oracle
the expectations were derived from) still say 23.1.4. Rewriting them without
re-measuring would falsify the record. The two generated ones are re-derivation
candidates — see §7.

Compiler Explorer serves **clang 23.1.0** (`cclang2310`) as its newest 23.x;
there is no 23.1.4/23.1.5 build. `scripts/godbolt.py`'s pin therefore stays
(`audit` reports `clang cclang2310 23.1.0 23.1.0 ok`), and the file now carries
a comment stating that the godbolt pin (23.1.0) and the repository pin (23.1.5)
are different binaries, so a table must record which one produced it.

The linker oracles need **no** change and were verified rather than assumed:
`tools/linker/setup_oracles.sh` pins `LLD_MINOR_PIN=23.1` and matches it as
`*" $LLD_MINOR_PIN."*`, which admits any 23.1.x including 23.1.5; mold stays
3.0.0 built with the owner's `--no-default-features --features x86_64,i386`
preset (`tests/linker/setup_oracles.sh` adds `system-allocator`). Both
preferences are untouched and still enforced.

---

## 2. GDB 18.1 (`-march=haswell`) added to the pinned set

**Why gdb belongs here at all.** Two gates drive a real debugger, and both
resolve it **opportunistically** — verified, not assumed, by grepping for the
only two resolution sites in the tree:

| Gate | Resolution | Behaviour when absent |
|---|---|---|
| `tests/regression/check_dwarf_line_tables.py` (lines 303, 317) | `shutil.which("gdb")` | prints "gdb not installed, debugger check skipped" (line 701) **and still exits 0** |
| `tests/regression/check_incoming_byval_homes.sh` (line 28) | `command -v gdb` | skips the backtrace branch |

The first is the serious one: `break <fn>` must land on the declared file:line
and `break <file>:<line>` must resolve back through the line table, and on a
host without gdb that gate reports green having never asked a debugger
anything. Pinning gdb closes the hole instead of documenting it.
(`check_dwarf_versions.py` mentions gdb only in a comment about DWARF
constant-class consumers; it does not run one, so it is *not* a consumer of
this pin.)

**New file: `scripts/ci_install_gdb_oracle.sh`.** Pins GDB **18.1** (latest GNU
release, 2026-09-25) and builds it from source with
`CFLAGS="-O2 -march=haswell -mtune=haswell"`, because the owner's own packages
and LCCC's x86-64 output target the Haswell ISA level on a Raptor Lake host —
a generic-baseline debugger is not the binary this project ships or debugs
with. Only `all-gdb` is built (bfd/opcodes/libiberty come along; no assembler,
linker, simulator or gprofng), `MAKEINFO=true` keeps texinfo out of the
dependency set, and `--with-python=no` keeps the oracle independent of the
host's Python ABI (the gates use `-ex` scripts, not Python scripting).

GNU publishes **no checksum** for gdb tarballs, only a detached `.sig`, so the
pin was computed and **cross-verified byte-for-byte against both GNU mirrors**:

```
https://ftp.gnu.org/gnu/gdb/gdb-18.1.tar.xz          -> cd9fc3fe2b47743840e42c1592d3d87f8302eb18639c0b8b4ba0898002e2348f
https://sourceware.org/pub/gdb/releases/gdb-18.1.tar.xz -> cd9fc3fe2b47743840e42c1592d3d87f8302eb18639c0b8b4ba0898002e2348f
22,158,720 bytes, `cmp` byte-identical
```

Both mirrors are in the installer's transport list, so the second is a
fallback, never a different source of truth.

**The `-march` claim is verified on the artifact, not just recorded.** The
build stamp (`$prefix/.lccc-build`: version, source sha256, size, march,
cflags, compiler, banner, timestamp) is required for reuse, and `isa_census_ok`
disassembles the installed binary and requires **both** AVX2 and BMI2
encodings. Measured on the 18.1 build: `vinserti128` ×128, `bzhi` ×40,
`vfmadd` ×1, `vpmaskmovd` ×1 → the census reports **150 AVX2 and 341 BMI2
encodings**. A generic-baseline build contains neither, so a warm distro gdb
cannot pass as this oracle. `objdump` failure fails **closed** (§5.5).

**Wiring.** `oracle_toolchains_from_tags.sh` provisions it
(`$PREFIX/gdb-18.1`, `gdb` shim, `GDB=` in the generated `env.sh`, and it is
part of the offline `--verify-only` probe set); `arena_provision.py` gained
`--with-gdb` (opt-in: it is a multi-minute build, and only debug-info gates
need it); `check_toolchain_cache_freshness.py` pins `gdb` offline (version,
64-hex digest, `-march` present in the effective CFLAGS, and
`oracle_toolchains_from_tags.sh` agreeing) and online
(`parse_gdb_listing` over `sourceware.org/pub/gdb/releases/`).

---

## 3. Files changed (mechanism summary)

* **Pins:** `ci_install_clang_oracle.sh`, `oracle_toolchains_from_tags.sh`,
  `ci_install_gdb_oracle.sh` *(new)*, `restore_toolchain_cache.py`,
  `publish_toolchain_cache.py`, `publish_agent_toolchain_cache.sh`,
  `arena_provision.py`, `check_toolchain_cache_freshness.py`, `godbolt.py`.
* **Gates/CI:** `tests/regression/lib/clang_oracle.sh`, `ci_verify_oracles.sh`,
  `ci_local.sh`, `.github/workflows/{ci,compiler-research,ipa-lrs}.yml`,
  `.github/ci-lane-plan.json`, and every `tests/regression/check_*` that
  verifies `clang -dumpversion` / `clang version …`.
* **Tests:** `tests/corpus/test_toolchain_cache.py` (+8 tests),
  `tests/corpus/test_sse34_clang.py`,
  `tests/corpus/test_arena_provision_extraction.py`,
  `tests/regression/test_check_switch_cross_execute.py` and the mock banners
  in `check_ci_link_oracle_provisioning.py`.
* **Docs:** `engineering/toolchains/README.md` (23.1.5 audit section,
  corrected lock wording, **measured** network-reachability section),
  `scripts/README.md`, this file.

### 3.1 Round 2 (QEMU / kernel re-pin, MSRV removal) — 15 files

* **Re-pins:** `scripts/ensure_qemu_user.sh` (11.1.3 + both digests + a new
  self-test case that rejects the superseded 11.1.2),
  `tests/linker/setup_kernel_tools.sh` (`PINNED_KVER=v7.3.1`, and `BASE` moved
  to the **stable** kernel.org tree because `v7.3.1` does not exist in
  `torvalds/linux.git`).
* **Drift-proofed:** `scripts/publish_agent_toolchain_cache.sh` now *derives*
  the QEMU version/`.deb`/digests and the kernel kver/tree/pin-table from those
  two enforcing scripts instead of carrying its own copy — the duplicated
  table is what let this cache hold mold 2.42.1 for three weeks.
* **MSRV removal:** `Cargo.toml` (no `rust-version`),
  `.github/workflows/ci.yml` (the `msrv` job and its `needs:` entry),
  `scripts/ci_local.sh` (the `msrv-check` gate, `--skip-msrv`, `SKIP_MSRV`),
  `engineering/toolchains/agent-cache-lock.json` (the `rust-msrv` entry and
  priority slot), `scripts/publish_agent_toolchain_cache.sh` (its payload),
  `scripts/restore_agent_toolchain_cache.py` + `scripts/restore_toolchain_cache.py`
  (comments), `scripts/clippy_msrv_lints.txt` + `scripts/check_clippy_policy.py`
  (re-documented: the typo-guard stays, the MSRV rationale does not).
* **Enforcement:** `scripts/check_toolchain_cache_freshness.py` — the
  `rust-msrv` pin and INFO row are gone, two new *asserted* superseded-payload
  facts were added (`qemu-agent-payload` 11.1.2, `kernel-relocs-agent-payload`
  7.3), and a new offline check fails if a `rust-version` key or any 1.97.0
  reference comes back.
* **Tests:** `tests/corpus/test_toolchain_cache.py`,
  `tests/corpus/test_parallel_ci_layout.py`, `scripts/test_ci_resources.py`.
* **Docs:** `engineering/toolchains/README.md`, `engineering/CI-PIPELINE.md`
  (annotated, not re-measured), this file.

---

## 4. Validation actually run (smoke-only, per instruction)

| Check | Command | Result |
|---|---|---|
| Offline pin consistency | `check_toolchain_cache_freshness.py --offline` | **exit 0**, 13 pins CURRENT, 4 INFO rows |
| Online audit (real upstream) | `check_toolchain_cache_freshness.py` | clang 23.1.5 CURRENT + asset-digest cross-check silent; **gdb 18.1 CURRENT (latest upstream 18.1)**; 2 pre-existing STALE rows (§6) |
| Corpus unit gate (the repo's own invocation, `ci_local.sh:2481`) | `python3 -m unittest discover -s tests/corpus -p 'test_*.py'` | **1203 tests, OK** |
| …same gate on pristine base | idem on `f530c231` | **1195 tests, 1 ERROR, FAILED** → no regressions, +8 tests, 1 pre-existing failure fixed |
| Oracle provisioning self-tests | `ci_verify_oracles.sh --self-test` | **10/10 pass** (incl. the 23.1.5 pin and CLANG-override cases) |
| GDB installer self-test | `ci_install_gdb_oracle.sh --self-test` | **8/8 cases pass** |
| Real Clang install | `TMPDIR=/var/tmp ci_install_clang_oracle.sh --prefix …` | **exit 0**; 1,198,896,522 B verified; `clang -dumpversion` = 23.1.5 |
| The repo's own oracle contract | `. tests/regression/lib/clang_oracle.sh; require_clang_oracle` | resolves the 23.1.5 binary; **negative case** rejects the host clang 14 with "the oracle is Clang 23.1.5 exactly" |
| Real GDB build + verify | `TMPDIR=/var/tmp ci_install_gdb_oracle.sh --prefix …` | banner `GNU gdb (GDB) 18.1`; stamp complete; **ISA census 150 AVX2 / 341 BMI2**; DWARF probe resolves `break main` **and** `info line main` **and** `break probe.c:2` |
| CI lane-plan contract | `tests/regression/check_ci_lane_plan.py` | **PASS** (2938 s of gate cost, lanes pinning the oracle all install it) |
| Godbolt oracle audit | `scripts/godbolt.py audit` | **exit 0**, all pinned aliases current (CE clang = 23.1.0) |
| Differential smoke | clang 23.1.5 vs host gcc 14.2 on a zlib-ng-style match loop | both compile; pipeline yields comparable instruction counts (117 vs 49) — *pipeline smoke test, not a codegen claim* |

**Round 2** (same commands, after the re-pins and the MSRV removal):

| Check | Command | Result |
|---|---|---|
| Online audit | `check_toolchain_cache_freshness.py` | **exit 0** — 13/13 CURRENT against live upstream, 0 STALE, 5 asserted-superseded INFO rows |
| Offline pin consistency | `--offline` | **exit 0**, no BROKEN |
| The two re-pinned packages, verified against upstream | audit output | `qemu 11.1.3 (latest upstream: 11.1.3)`, `kernel-relocs 7.3.1 (latest upstream: 7.3.1)` |
| QEMU `.deb` digest, end to end | downloaded the real 72,595,524 B `.deb`, `sha256sum --check` | **OK**; `dpkg-deb -c` confirms all four emulators; `Built-Using` gcc-16 16.2.0-4 |
| QEMU installer self-test | `ensure_qemu_user.sh --self-test` | **PASS** — accepts 11.1.3; rejects 8.2.2, the superseded 11.1.2, an 11.1.20 lookalike and an incomplete set |
| Kernel relocs, real provision | `setup_kernel_tools.sh --prefix /var/tmp/ktools` | **exit 0**; all six pins verified from the **stable** tree; `relocs` built and runs; stamp `v7.3.1 sha256:fbccadf6…` |
| Corpus unit gate | `python3 -m unittest discover -s tests/corpus -p 'test_*.py'` | **1203 tests, OK (skipped=15)** — same count as round 1 (one MSRV test replaced by a retirement test) |
| Gate parity | `check_ci_gate_parity.py` | **PASS** (540 commands, 27 invocation contracts) |
| CI layout / lane split / resources | `test_parallel_ci_layout.py`, `test_ci_lane_split.py`, `test_ci_resources.py` | 15 OK / 13 OK / 5 OK |
| Toolchain-cache tests | `test_toolchain_cache.py` | **26/26 OK** |
| ci.yml integrity | YAML parse + `needs` graph walk | **18 jobs**, `msrv` absent, **no dangling `needs`** |

Note on one misleading run: invoking the corpus gate with `TMPDIR=/var/tmp`
produces 3 failures in `test_offline_deb_extract.py`. That is the *invocation*,
not this patch — the test deliberately refuses extraction destinations outside
`/home/user` or `/tmp`, and it passes with the default `TMPDIR` (proved by
running that file both ways; the file is not in this diff).

Not run (and not claimed): `ci_local.sh --fast`/`--slow`, the hosted publisher
workflows (need `gh` + docker + push), the full `ext_vector_type`/SIMD matrices
(need a built `lccc`), and any performance measurement. **No Rust/lccc build
was performed** — this patch touches no Rust code, and the standing `-O1 -j2
fastbuild` build remains the next session's first step before any codegen work.

---

## 5. Defects found by testing (each fixed in this patch)

### 5.1 The escaped-regex half-bump (the dangerous one)
A textual `23.1.4 → 23.1.5` sweep does **not** match `23\.1\.4` inside regex
literals, so 18 live oracle-identity checks kept demanding 23.1.4 while their
error messages claimed 23.1.5 — e.g.
`check_sse34_clang.py` failed with *"require identified Clang 23.1.5, got
clang version 23.1.5"*. Every gate comparing `clang --version` would have
red-failed against the very oracle it asked for. Fixed by a second sweep over
the escaped spelling plus a broader `23[^0-9]{1,4}1[^0-9]{1,4}4` audit; the
only remaining hits are intentional superseded-payload notes and dated
evidence. **Lesson for future bumps: sweep `X\.Y\.Z`, `X\.Y\.Z` in regexes, and
any bracket/escaped spelling; then run the corpus gate, which is what caught
this.**

### 5.2 `curl` exit 23 on a small tmpfs, misread as a network failure
The 1.2 GiB asset cannot fit in this harness's 992 MiB `/tmp`, and curl dies
part-way with *"write error"*. Both installers now preflight the download
directory's free space (size taken from a `HEAD` request, so the number is the
real one) and fail with the fix named:
*"/tmp/clang-oracle.XXXX has 990 MiB free; downloading LLVM-23.1.5-Linux-X64.tar.zst needs ~1457 MiB. Point TMPDIR at a disk-backed directory, e.g. TMPDIR=/var/tmp."*
Verified by running it against `/tmp` and reading that exact message.

### 5.3 `ci_install_clang_oracle.sh --version …` was rejected outright
`.github/workflows/compiler-research.yml` calls the installer with
`--version 23.1.5`, but the argument parser answered *"unknown argument"* and
exited 2 — that lane could never have run. The flag is now accepted **and
verified against the pin** (a mismatch exits 2), so a workflow that names a
version fails at provisioning instead of silently installing a different one.

### 5.4 The "release-asset CDN is unreachable" assumption was stale
`engineering/toolchains/README.md` stated the Arena allowlist blocks
`release-assets.githubusercontent.com`, which is why `arena_provision.py`
detoured through a git blob. Re-tested: a ranged GET returns **HTTP 206** and
the full asset downloads. The blob detour could therefore only ever serve a
superseded oracle; it is gone, and the README now records the measurement
instead of the assumption.

### 5.5 Two fail-open paths in code written this session, both caught by tests
* `isa_census_ok` returned success when `objdump` failed, so a stub binary
  "passed" the Haswell check. Now fails closed and says the claim is
  unverifiable. `--self-test` covers it.
* the first `dwarf_probe` matched only `info line` and passed it **no
  argument**, which answers "No line number information available" in batch
  mode — the probe failed against a perfectly good debugger. It now asserts
  both directions against the strings gdb 18.1 actually prints.

### 5.6 Missing GMP/MPFR diagnosed late
GDB's configure died with *"Building GDB requires GMP 4.2+, and MPFR 3.1.0+"*
after the download and unpack, which reads like a GDB bug. The installer's
header preflight now covers `readline/readline.h zlib.h gmp.h mpfr.h` and names
the packages to install.

### 5.7 Pre-existing, unrelated, fixed anyway
`tests/corpus/test_arena_provision_extraction.py` errored on Python 3.13
(`tarfile.addfile` refuses a non-zero-size member with no fileobj) **before
exercising any policy**. Proven pre-existing by running the module on pristine
`f530c231` (2 errors) versus this tree (11/11 OK). The helper now pads the
content with NULs; the unsafe-member cases only exercise the name/type policy.

---

## 6. Round 2 — the two stale pins fixed, and the Rust MSRV floor retired

### 6.1 QEMU user 11.1.2 → 11.1.3

`scripts/ensure_qemu_user.sh` now pins `11.1.3` / `11.1.3+ds-1`, with
`DEB_SHA256=5fa915e759f314400cf7d64a2745747e6baa50a180c938b3e8d281ce57ede452`
and `DEB_SHA1=df5ab4a6de0ab31b9a5215eb03929285dcbe4d7e`.

How those were established, not asserted:

* the SHA-256 comes from the **signed** Debian sid `Packages` index
  (`dists/sid/main/binary-amd64/Packages.xz`, `qemu-user` stanza,
  `Size: 72595524`);
* then all 72,595,524 bytes were actually downloaded and
  `sha256sum --check` passed against that index value;
* `dpkg-deb -f` records `Built-Using: gcc-16 (= 16.2.0-4)` — so this `.deb` is
  *not* the gcc-12 build round 1's `debs-bookworm` snapshot pairs with; it is
  the current sid build, which is the point of pinning sid;
* `dpkg-deb -c` confirms the four emulators the suite drives
  (`qemu-{aarch64,arm,i386,riscv64}`) are all inside;
* the SHA-1 is the same file's SHA-1, computed locally because the index
  carries only MD5 and SHA-256 — it is used solely as the
  `snapshot.debian.org/file/<sha1>` fallback key.

The `--self-test` gained a case for the **superseded** pin: a warm 11.1.2
prefix must not vouch for itself after a bump. That is the whole reason the
version probe is exact rather than "new enough".

### 6.2 kernel `relocs` v7.3 → v7.3.1 — a label change, not a content change

`PINNED_KVER=v7.3.1`. **All six source digests are byte-identical to the v7.3
pins** — nothing under `arch/x86/tools/` moved in the point release — so the
pin table is unchanged and says so in a comment. Verified by fetching all six
files at `v7.3.1` over **two independent transports** (kernel.org's stable
cgit and the `gregkh/linux` GitHub mirror) and comparing digests pairwise;
all six agreed.

One real discovery: `v7.3.1` **does not exist in `torvalds/linux.git`**. The
same cgit `plain` fetch answers 404 for `?h=v7.3.1` on the torvalds tree and
200 on the stable tree, so `BASE` moved to
`git.kernel.org/pub/scm/linux/kernel/git/stable/linux.git/plain`. The stable
tree serves mainline tags too, so it covers every `--kver` the script accepts.
Had the bump been done by editing only `PINNED_KVER`, the fetch would have
failed with a 404 that looks like a network outage.

Provisioned for real, not simulated: `setup_kernel_tools.sh --prefix …` fetched
all six files, verified every digest, built `relocs`, wrote the
`v7.3.1 sha256:fbccadf6…` stamp, and the resulting binary runs.

### 6.3 The publisher no longer carries its own copy of either pin

`scripts/publish_agent_toolchain_cache.sh` used to hardcode the `.deb`
filename, the snapshot SHA-1, the tarball name, the kernel `BASE` URL **and a
second copy of the six-digest pin table**. Every one of those would have
silently kept publishing 11.1.2/v7.3 after this patch. It now reads the
version, digests, kver, tree and pin table straight out of the two enforcing
scripts (the pins via a small Python read of the same `PINS` block), so a
re-pin cannot leave the publisher behind — the exact drift that let this cache
hold mold 2.42.1 for three weeks.

### 6.4 The Rust MSRV floor is gone — and one correction to the request

The request was to delete "the old Rust **1.97.1** toolchain". The pin was
actually **1.97.0** (`Cargo.toml:17`, `rust-version = "1.97.0"`, an
AUD-TOOLCHAIN-1 measurement from 2026-10-27); there is no 1.97.1 anywhere in
the tree. Everything that constituted the floor was removed:

| Removed | Where |
|---|---|
| `rust-version = "1.97.0"` | `Cargo.toml` |
| the hosted `msrv` job (`dtolnay/rust-toolchain@1.97.0`, `cargo check --locked`) and its `- msrv` entry in the aggregator's `needs:` | `.github/workflows/ci.yml` — 19 jobs → **18** |
| the `msrv-check` gate, `msrv_check_gate`, the `--skip-msrv` flag, `SKIP_MSRV` and its help text | `scripts/ci_local.sh` |
| the `rust-cache-1.97.0` transport tag, the MSRV component materialisation and the `rust-msrv` `CACHE.json` entry | `scripts/publish_agent_toolchain_cache.sh` |
| the `rust-msrv` tool entry (129,382,312 B payload) and its priority slot | `engineering/toolchains/agent-cache-lock.json` |
| the `rust-msrv` pin and INFO row | `scripts/check_toolchain_cache_freshness.py` |
| the MSRV-lane assertions | `test_parallel_ci_layout.py` (`PRODUCERS`), `test_ci_resources.py`, `test_toolchain_cache.py` |

Two things were **kept deliberately**:

* `scripts/clippy_msrv_lints.txt` and `check_clippy_policy.py`'s unknown-lints
  audit. The `#![allow(unknown_lints)]` mechanism exists because a *developer's*
  Clippy can be older than the newest lint the source mentions
  (`clippy::chunks_exact_to_as_chunks` needs Clippy ≥ 1.98) — that hazard is
  independent of any floor. The filename is historical, both readers are
  re-documented, and the typo-guard still fails on a policy-list misspelling.
* `cargo check --locked` on the pinned toolchain, via the `build`/`debug`/
  `clippy`/`rustfmt` lanes. The floor is gone; "a Cargo.lock that does not
  resolve on current Rust must fail loudly" is not.

And one thing was **added**, because a deletion that nothing enforces will be
undone by the next well-meaning patch: `check_toolchain_cache_freshness.py`
now fails if `Cargo.toml` grows a `rust-version` key again, or if any live
script, workflow or cache lock mentions `rust-cache-1.97.0`,
`rust-1.97.0-msrv`, `LCCC_MSRV_TOOLCHAIN`, `msrv_check_gate`, `cargo +1.97.0`,
`rust-toolchain@1.97.0` or `"rust-msrv"`. Dated evidence and engineering
reports are excluded from the scan — they record what was true then.

**Not validated here:** no Rust toolchain exists in this sandbox, so
`cargo check`, `clippy` and `rustfmt` could not be run. `check_clippy_policy.py`
exits 127 (`cargo is not on PATH`). The MSRV removal is therefore verified
*structurally* (YAML job graph, gate registry, lock schema, parity checker,
1203-test corpus) but **not** by a Rust build.

---

## 7. To-do for the next session (prioritised)

1. **Build lccc** (`fastbuild` preset, `-O1`, `-j2`, swap active) and record a
   `ci_local.sh --fast` pass, then re-snapshot so the deliverable stops being
   `ci_gate=UNGATED`.
2. **Re-derive the two generated Clang artifacts against 23.1.5** and drop
   their 23.1.4 provenance headers: `tests/regression/sse2_codegen_matrix.json`
   (SIMD matrix run) and `tests/regression/ext_vector_abi/grid_expectations.txt`
   (two hosted `ext_vector_type` ABI runs). Until then those two files still
   document a 23.1.4 measurement, and the ABI gates run against 23.1.5 — a
   point release should not move an ABI, but the expectation files should say
   which oracle produced them.
3. **Make the DWARF gates fail instead of skip** when the pinned gdb is absent
   (they are the reason gdb is pinned): add a `tests/regression/lib/gdb_oracle.sh`
   in the shape of `clang_oracle.sh` (`$GDB` → `gdb` on PATH, exact 18.1, no
   silent substitute) and switch the two real consumers —
   `check_dwarf_line_tables.py` (both `shutil.which` sites) and
   `check_incoming_byval_homes.sh` — to it. Keep a
   `--allow-missing-debugger` escape for hosts that genuinely have none, so the
   skip stays *explicit* rather than default.
4. **Run the hosted publishers** (`refresh_toolchain_cache`,
   `Publish GitHub-only agent toolchain cache`), then retire the
   `clang-cache-asset`/`clang-agent-payload` exceptions exactly as §1.1 lists.
5. **Consider a prebuilt gdb in the agent cache** (`harness-cache-7`): it would
   save a multi-minute source build per sandbox. Recipe: build with
   `ci_install_gdb_oracle.sh` inside the bookworm container the publisher
   already uses (needs `libgmp-dev libmpfr-dev libreadline-dev zlib1g-dev`),
   pack `gdb-18.1-linux-x64.tar.xz`, add it to `CACHE.json`'s tools/priority
   and to `agent-cache-lock.json`. Deliberately **not** written this session:
   the publisher needs `gh`+docker+push, so it could not be validated, and an
   untested 40-line docker build in a hosted publisher is a worse trade than a
   verified 10-minute local build.
6. ~~Fix the qemu/kernel staleness~~ — **done in round 2** (§6.1–§6.3).
   What remains is the *publishing* half: run the agent-cache publisher so
   `harness-cache-7` actually carries QEMU 11.1.3 and relocs v7.3.1, then
   delete the `qemu-agent-payload` / `kernel-relocs-agent-payload` assertions
   (they will fail the moment the lock is updated, which is the intent).
8. **First Rust build after the MSRV removal** (§6.4): `cargo check --locked`
   on 1.99.1, plus `clippy` and `rustfmt`, to confirm the floor was not
   load-bearing for anything the tree still compiles. Nothing in this sandbox
   has a Rust toolchain, so that is untested here.
9. **Now that the floor is gone, new language features are fair game** — but
   each one needs a lane that fails if it regresses. `cargo check --locked` on
   the pinned toolchain is that lane; do not add features that require a newer
   compiler than the one `ci_local.sh` pins without moving the pin there.
7. **Compiler Explorer lag:** CE's newest Clang is 23.1.0 while the repository
   pin is 23.1.5. Any "we match Clang" table should record which of the two
   produced it; `godbolt.py` now says so at the alias table.

---

## 8. Environment notes worth keeping

* **Swap is mandatory here.** 1.9 GiB RAM with no swap OOM-kills anything
  larger than a single rustc; a 6 GiB `/swapfile` was installed at session
  start and verified with `swapon --show`.
* **`/tmp` is a 992 MiB tmpfs.** Anything that downloads or builds must set
  `TMPDIR=/var/tmp` (disk-backed, on the 25 GiB root). This bit the Clang
  installer (§5.2) and would bite the gdb build (~2 GiB tree).
* **Keep the checkout out of the persisted workspace.** The clone is ~234 MB /
  ~10 k files, which is at the harness snapshot cap; it lives in `/var/tmp/lccc`
  and only the patch, ledger and small artifacts are published to `/home/user`
  (`workspace: 8 files, 269 KB` at S01).
