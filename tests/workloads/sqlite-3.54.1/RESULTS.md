# SQLite 3.54.1 validation record — 2026-11-25–26 UTC

This is a reproducible **bounded** workload, not a certification of every
SQLite feature, every target runtime or peak hardware efficiency. Commands,
inputs and gates are in [README.md](README.md); raw logs/JSON were generated
under `target/sqlite-3.54.1-{enabled,disabled,clang,oracle}/` and deliberately
excluded from Git. The first record below describes the earlier pushed head
`44b4a31`; the dated **forward-only follow-up** at the end describes the
subsequently validated compiler and supersedes its binary/performance numbers.
No failures were suppressed to obtain either record.

## Input and build identity

- Upstream `sqlite/sqlite` GitHub `version-3.54.1` tag ZIP SHA-256:
  `a9b05054c19e5649f11d8af849cb1fc1b2311eb6f211b96b244fb5d5e8368cef`;
  release `manifest.uuid`:
  `555e07ef33f69a0957d24d8e555b702464a131d6f1607f4c3c1e4551e8e6ca76`.
  The official sqlite.org ZIP was not downloaded; the verified upstream tag is
  a mirror of the release, not a claim of byte-identical ZIP archives.
- `ms178/archpkgbuilds` SQLite recipe commit
  `0516be29c004aa2a64e8a66ba84a3eda154ccd6e` targets **3.54**.
  Its Lemon patch failed the *exact* 3.54.1 dry run; the release-specific
  equivalent was strictly applied with `--fuzz=0`. The recipe's CPP defines
  were read from the pinned PKGBUILD. Clang PGO/BOLT steps were not attempted.
- LCCC and standalone `lccc-ld`: `fastbuild` (`-O1`, two Cargo jobs), with an
  active **8 GiB** swapfile. The same LCCC binary SHA-256
  `2201b731b3232c86c86ee5d642d272478923d3431ca7619dd6f08d8a76b12b5c`
  compiled **both** final A/B builds. SQLite C flags in both builds:
  `-O2 -march=x86-64-v3 -mtune=raptorlake -fomit-frame-pointer`.
- GCC **16.2.0** (full-project reference) and Clang **23.1.5** (an additional
  full-project reference) were available locally. **ICX 2026.2 was not**
  installed here; the repository's authenticated hosted ICX integration is a
  separate check, not a local SQLite/ICX comparison.

## Correctness and diagnostics

- The enabled and `CCC_DISABLE_PASSES=ptr_delta_phi` control runs **both
  passed**: LCCC/GCC/standalone-`lccc-ld` SQLite CLI builds, `relational.sql`
  and `features.sql` byte-for-byte equal to GCC, all `PRAGMA integrity_check`
  results, and six upstream `speedtest1 --verify` sets (`main`, `cte`, `rtree`,
  `orm`, `json`, `fp`). Each build's configure, compile, speedtest compile and
  standalone link logs passed the automated **zero-warning** gate.
- On the same pinned/patched source, Clang 23.1.5's SQLite CLI and static library
  **passed** the zero-warning gate, both SQL cases against GCC, all six
  verification hashes and test identities, and per-set database integrity.
  Clang's one-run VM times are a correctness screen, not stable comparative
  performance measurements.
- The final-binary x86-64 fixture passed **90,000 independently decoded varints
  and 4,096 guarded negative controls × four builds** (GCC, Clang, LCCC
  enabled, LCCC disabled), including `CCC_VERIFY_IR=abort CCC_VALIDATE_SSA=1`;
  no compiler warnings under `-Werror`. On x86-64, i686, AArch64 and RISC-V 64,
  pass-on/off and reference compilation passed the warning/IR/SSA gates. The
  **varint** pointer subtraction disappeared only with the pass; the negative
  control was unchanged in both modes on all four targets. On this Linux/x86-64
  host, freestanding **i686** binaries passed **90,000 varints + 50,000 guarded
  deltas × GCC/Clang/LCCC enabled/LCCC disabled** against a separately
  GCC-compiled oracle, without a multilib C runtime. Pinned QEMU-user **11.1.3**
  then executed freestanding **AArch64 and RISC-V 64** binaries: the same
  140,000 cases × Clang/LCCC enabled/LCCC disabled per target, each against an
  independent Clang-compiled oracle. All six emulated runs passed. Cross-target
  *native* speed was not measured, and no such speedup is claimed.
- Local compiler unit suite: **12/12** pointer-delta tests passed (including
  malformed SSA, negative/out-of-range offsets, non-word-width rejection,
  commuted `Add`, a guarded-backward-step negative control, and shared phis).
  Summarizer/comparator tests: **9/9**. Audit census passed without an
  increased baseline (`panic_sites=856`, `fns_over_300=232`). An additional
  one-round `main`/size-5 SQLite smoke after the database-cleanup change
  passed SQL, hash and integrity gates, and left **zero** transient databases;
  its VM time was not used for the performance table.

## Performance: separate targeted and whole-project questions

`A` is LCCC with the pass, `B` is the **same compiler** with only
`ptr_delta_phi` disabled. Paired SQLite uses seven alternating AB/BA rounds
per set, fresh database and integrity check **per run**, and rejects any hash,
test-identity, source, compiler or flags mismatch before saving a PASS report.
All 84 paired runs passed. Ratios below are **B/A**; values above one favor A.
A/GCC and B/GCC came from separately validated three-run builds, **not** the
seven-round paired A/B schedule; do not subtract those unpaired columns to
infer a speedup (the R-tree rows show why that inference would be unsafe).

| speedtest1 set | A/GCC, 3-run VM median | B/GCC, 3-run VM median | B/A, 7-run VM medians | median paired B/A | B slower in pairs |
|---|---:|---:|---:|---:|---:|
| main | 1.315 | 1.314 | 1.001 | 0.999 | 3/7 |
| cte | 1.353 | 1.442 | 1.009 | 1.017 | 5/7 |
| rtree | 1.551 | 1.663 | 0.979 | 0.991 | 1/7 |
| orm | 1.176 | 1.302 | 1.000 | 1.010 | 4/7 |
| json | 1.135 | 1.245 | 0.994 | 0.994 | 2/7 |
| fp | 1.423 | 1.384 | 1.007 | 1.020 | 4/7 |

The dedicated *cache-resident* 4096-input varint workload first checks an
independent 90,000-case oracle in each executable. On the **final** compiler,
in **15 interleaved pairs**, A's minimum VM wall time was **111.49 ms**,
B's **115.84 ms** (B/A **1.0390**; low-three B/A **1.0509**). This supports a
targeted improvement in this environment, **not** a SQLite-wide win:
whole-project paired deltas were small, inconsistent or noisy. The measured
SQLite `sqlite3GetVarint` object function shrank from **409 to 404 bytes**
with the pass; target-specific isolated assembly also eliminated its pointer
subtraction on all four targets.
Neither assembly size nor shared-VM times imply a native Raptor Lake PMU result.

### Rejected candidate: a guarded backwards pointer step

The separate [`ptr_delta_phi_negative.c`](ptr_delta_phi_negative.c) has a
`p - 7` arm, a `p + 11` arm, and a base arm. A temporary, **not shipped**
extension proved a modular constant-offset `Sub(base, 7)` mathematically and
passed the four-ISA functional oracles, but its x86-64 function fell only from
21 to 19 assembly instructions. Against the same compiler with that extension
disabled, the cache-resident
[`ptr_delta_phi_negative_bench.c`](ptr_delta_phi_negative_bench.c) instead
measured **B/A = 0.939** (15 interleaved pairs, 13/15 favor B) and **0.932**
(31 pairs, 25/31 favor B) by the harness's minimum VM elapsed time; the
31-pair *median paired* B/A was **0.982**. Below one **favors the disabled
control**, so the apparent code-size win did *not* establish a host runtime win.
The candidate was reverted. In the delivered pass this source is a negative
control: both pass settings retain the pointer subtraction and emit identical
function assembly on all four targets. The delivered compiler's **15-pair**
negative-control screen measured B/A **0.992** by minimum and **1.002** by
low-three mean, consistent with no measurable change. This decision does not
establish that backwards steps are always slower on physical hardware; it
avoids shipping a measured local regression without a backend-aware solution.
The exploratory candidate's raw JSON is in
`target/sqlite-3.54.1-oracle/negative-ab-31.json`
(not versioned); rerunning the final source alone cannot reproduce the
unshipped candidate's timing.

Limits: SQLite's full Tcl `make quicktest` suite did **not** run (`tclsh` was
absent); an official sqlite.org ZIP, a local ICX SQLite build, native AArch64
and RISC-V 64 **native** execution, PMU counters, and a broad statistical
performance claim are **not** part of this record. GitHub CI status must be checked on the
final PR head separately, not inferred from these local workload results.

## 2026-11-26 forward-only follow-up — replacement compiler and deeper red-team screen

The previous gate excluded only `Sub(base, 7)`. An additional *unshipped*
31-pair screen showed that its equivalent `p += -7` lowers as GEP/Add with a
negative constant, **was still folded**, and regressed on this host too:
**B/A = 0.9495** by minimum, **0.9585** by low-three, and **0.9617** by
median of paired elapsed-time ratios; the pass-disabled B arm was faster in
**25/31** pairs. Enabled/disabled function assembly differed on x86-64, i686,
AArch64 and RISC-V 64. This is one guarded input and one VM, not evidence that
all negative offsets are slower. The delivered pass now rejects *both* negative
GEP/Add and backward Sub arms, retaining zero and positive offsets. Its Rust
unit tests include signed-I32/I64 GEP through copies, commuted negative Add on
LP64/ILP32, and an unchanged Sub control. The new
[`ptr_delta_phi_negative_add.c`](ptr_delta_phi_negative_add.c) fixture has its
own host benchmark and four-target freestanding oracle.

The replacement LCCC fastbuild SHA-256 is
`fc9dab34720698341f8741a7b828917838dec899cdc6273c305b4db1503cbd55`;
standalone `lccc-ld` is
`a83050db13bd6a2c513b8c0b73395fd7415f93542dfc842a1dcc82c85166c4f5`.
Both SQLite arms report that **same** LCCC digest and the sole pass toggle;
GCC 16.2 and Clang 23.1.5 use the same pinned/patched 3.54.1 release and C
flags as the earlier record. Enabled and disabled **both passed** the full
bounded GCC/LCCC/lccc-ld SQL and six-speedtest correctness, hash, identity,
warning and database-integrity gates; the Clang reference passed both SQL
cases and all six speedtest hashes and integrity checks. Every one of the **84**
seven-round paired SQLite samples passed its hash, identity, empty-stderr and
fresh-database integrity gates. The replacement compiler passed **14/14**
pass-specific Rust tests; the updated comparator/runner and A/B harness tests
passed **17/17** and **10/10**. Host GCC/Clang/LCCC on/off passed 90,000
varints plus 4,096 inputs for **each** backward encoding. On i686 and pinned
QEMU 11.1.3 AArch64/RISC-V 64, the independent-oracle binaries passed 90,000
varints plus **100,000** guarded deltas per compiler/mode, while pass-on/off
emitted identical negative-control functions on each target. GDB/nm confirmed
that the final SQLite `sqlite3GetVarint` is **404 bytes** with the pass versus
**409** without it (no debug-type information was embedded in these builds).

The updated differential gates additionally reject unexpected runtime stderr
and stale PASS outputs, and the database-integrity shell gate now checks the
CLI **exit status** separately from its `ok` stdout (command substitution
alone could conceal a nonzero exit). Synthetic nonzero, stderr, corrupt and
successful CLI unit cases cover this guard; a real SQLite CLI also returned
`ok` with empty stderr. The previous full-size enabled/disabled runs exercised
its equivalent inline positive path. After extracting that guard into a
sourced/testable helper, a **final** one-round size-5 `main` SQLite build also
passed GCC/LCCC/lccc-ld SQL, the hash and both `integrity_check` calls with
zero stderr; its VM time was excluded from the performance table.

| speedtest1 set | A/GCC, 3-run VM median | B/GCC, 3-run VM median | B/A, 7-run VM medians | median paired B/A | B slower in pairs |
|---|---:|---:|---:|---:|---:|
| main | 1.265 | 1.285 | 1.046 | 1.036 | 6/7 |
| cte | 1.395 | 1.382 | 1.004 | 1.004 | 4/7 |
| rtree | 1.632 | 1.602 | 1.007 | 1.013 | 5/7 |
| orm | 1.200 | 1.217 | 1.034 | 0.990 | 3/7 |
| json | 1.160 | 1.106 | 1.037 | 1.016 | 4/7 |
| fp | 1.362 | 1.425 | 1.009 | 1.012 | 6/7 |

Here A is the forward-only pass; B is exactly that compiler with the pass
disabled. A/GCC and B/GCC were measured **outside** the seven-round paired
schedule. The 15-pair targeted varint screen gave minimum B/A **1.0358**,
low-three B/A **1.0252**, and **11/15** pairs slower with B. In separate
**31-pair** delivered-code negative controls, Sub-encoded backward steps had
minimum/low-three B/A **0.9929/1.0025** (17/31 B slower), and Add/GEP-encoded
steps **0.9953/1.0055** (15/31 B slower). On/off assembly for both controls
was identical; these small ratios are consistent with measurement noise.
The different whole-project paired ratios from the earlier head, and even the
opposite pooled versus paired ORM direction above, rule out a robust
whole-SQLite speedup claim on seven VM rounds. They also cannot justify a
native Raptor Lake, i686, AArch64 or RISC-V runtime-performance claim.

As a further bottleneck screen, a *pre-follow-up* size-1 Callgrind run with
matching verification hashes counted dynamic instruction references (not time):
`main` LCCC/GCC/Clang **180.6/112.2/117.0 million**, `cte`
**363.7/207.1/232.5 million**, `rtree` **308.0/173.5/184.9 million**,
`orm` **64.4/44.5/43.8 million**, `json` **82.0/55.9/54.8 million**, and
`fp` **180.8/111.7/121.2 million**. Around 50.9M versus 30.0M `main`
self-instructions landed in `sqlite3VdbeExec`, whose emitted frames were
`0xa38` (LCCC) versus `0x130` (GCC). Neither attribution nor frame size
isolates a causal fix, so no speculative backend rewrite was shipped.
GDB inspected the new decoder assembly; `objtool`/`pahole` were not available
locally, and the optimized SQLite objects lacked DWARF type layouts for a
meaningful `pahole` analysis. The full SQLite Tcl suite, local ICX SQLite,
native cross-target speed, and a completed full local fast-CI run remain
**unverified** here. Hosted CI must be assessed on the final pushed head, not
inferred from earlier green checks.
