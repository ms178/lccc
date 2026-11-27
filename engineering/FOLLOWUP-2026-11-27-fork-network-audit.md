# Fork-network audit — 2026-11-27

## Baseline and delivery contract

- Reviewed LCCC baseline: `7604253fa62bbce385b740ea33d7ce573f21cf5a` (`ms178/lccc/main`).
- No fork checkout, build script, dependency installer, workflow or executable
  was run. Forks were read as Git objects in a separate bare object store.
- Only adapted changes with local reproducers are admitted. A commit title,
  an adoption marker, a clean diff, or passing another fork's tests is not proof.
- Builds use `scripts/build_lccc_fast.sh`, explicit `LCCC_BUILD_JOBS=2`,
  `opt-level=1`, no LTO, in the foreground. Eight GiB of **active** swap was
  installed. Source/build/cache directories are outside the size-capped Arena
  workspace; only delivery artifacts belong in that workspace.
- `scripts/arena_snapshot.sh` publishes the cumulative patch after each validated
  change. Names: `ms178-1.<UTC timestamp>.<sequence>-<topic>.patch`, plus the
  rolling `ms178-1.patch` alias and `snapshots/LINEAGE.log`.
- Full CI was **not** run, as explicitly requested. Snapshot gate is correctly
  `UNGATED`, not a fabricated `ci_local --fast` pass. Targeted results follow.

## Discovery and exclusions

The API/branch census recorded 261 repositories including the original and
LCCC itself. All advertised branch heads were requested, not just default
branches: many valuable changes live only on topic branches. Excluding LCCC's
own historical branches, there were 150 distinct commit tips / 147 tree IDs.
Five repositories' branch reads remained unavailable after a retry. These are
coverage gaps, not an assertion that the repositories are clean or malicious.
The detailed admission review, exact source commits, exclusion evidence and
remaining optimizer candidates are in
[AUDIT-2026-11-27-fork-admission.md](AUDIT-2026-11-27-fork-admission.md).
The new non-executing `audit_fork_network.py` preserves both default and topic
branches, de-duplicates immutable tips, checks execution-boundary paths, and
supports `inspect --against <immutable-LCCC-commit>` for exact entry comparison.
Its eleven offline fixture tests pass; an inaccessible fork still makes the
live report return **2 / incomplete coverage**, never a fake clean audit.

A concrete exclusion is `wadsaek/claudes-c-compiler`, tip `bdaa75cad6`:
the subject says “Edit README.md”, but the changed path is the expression parser,
which substitutes unrelated text for string literals based on random input.
This semantic sabotage was identified by reading the diff; it was never built.
Do not import that code, its instructions, or its artifacts.

The large historical test campaigns and rewrites are not imported wholesale.
Exact source/tree inventories and semantic review have different evidentiary
strength; neither the census nor a filename/subject match certifies a fork.

## Validated adaptations

### 1. Preprocessor conditional common type and middle-operand grammar

Candidate: `sametyilmaztemel/claudes-c-compiler`, `5d3f86f8de`.

The old preprocessor selected an entire `(value, unsigned)` pair, so
`#if (1 ? -1 : 0U) < 0` chose the wrong source branch. Both arms determine
`?:`'s common type, even when one arm is inactive. Unlike C expression folding,
preprocessor integers use `intmax_t`/`uintmax_t` (C11 6.10.1p4).

The adaptation also handles a comma expression in the middle operand, without
changing right associativity or leaking `-Wundef` suppression out of inactive
arms. The existing C-expression ternary gate now checks the separate
preprocessor fixture as well.

- Baseline: 12 preprocessing errors on the fixture; GCC 16.2 / Clang 23.1.5
  both accept it.
- Fixed: all four LCCC target drivers preprocess with `-Wundef -Werror`.
- Existing runtime ternary gate: `-O0/-O1/-O2/-O3/-Os`, PASS.
- Targeted Rust preprocessor group: **15 passed**, including five new tests.

### 2. Compound assignment's signed source / unsigned common type

Candidate: `sametyilmaztemel/claudes-c-compiler`, `a50a017f26`.

`promote_for_op` zero-extended a negative signed LHS merely because the C
common type was unsigned. That conflated the C conversion width with the
backend's widened arithmetic carrier. It also lost signedness at the second
step of a 64-to-128-bit promotion.

The adapted fix preserves source signedness and uses two stages only when a
narrow signed value must first become an unsigned C value smaller than the
IR carrier: e.g. `short -> unsigned int -> i64`. Shift promotion remains
independent of RHS signedness.

- Baseline fixture fails; GCC 16.2 and Clang 23.1.5 pass.
- Fixed fixture passes at `-O0/-O1/-O2/-O3/-Os`.
- Cases cover 8/16/32/64/128-bit values, all 256 signed-byte inputs, volatile
  single-evaluation lvalues, signed controls and shift controls.
- Quotient/remainder results fit their signed destinations; the decisive
  assertions do not rely on implementation-defined out-of-range conversion.
- This fixes the ordinary scalar promotion helper. Bitfield, atomic, vector
  and complex-assignment lowering paths are not certified merely by sharing
  the words “compound assignment”; expand their independent tests before
  claiming the entire conversion family closed.

### 3. Point of declaration, with LCCC-specific scope preservation

Candidate: `sametyilmaztemel/claudes-c-compiler`, `c6e8d717b8`.

LCCC previously registered names at the end of the declaration, after all
initializers. `typedef char T; ... int T = sizeof(T);` therefore initialized
an `int` to 1, and `(T)-1` in a later comma sibling could become a cast rather
than subtraction.

The adaptation enters names at each completed declarator, keeps the existing
scope undo log (exactly one save of the old state), publishes typedef qualifier
metadata before later siblings, and preserves the inferred-type exception.
Local typedef size information stays sema-deferred: a spelling-only alias
cache cannot represent a captured outer binding after another local shadows
that spelling. A same-type file-scope redeclaration must not create `T -> T`
in the parser's size cache.

- Baseline reduced runtime example prints `1 151 -1`; GCC/Clang print `4 154 2`.
- Baseline also rejects the portable multi-declarator fixture.
- Fixed portable fixture: GCC/Clang/LCCC, five optimization levels: **15 PASS**.
- GNU inferred-typedef control: GCC/LCCC, five levels: **10 PASS**.
- Clang 23.1.5 rejects that GNU control; this is an explicit oracle
  disagreement, not a skipped test disguised as three-compiler agreement.
- Targeted Rust declaration group: **21 passed**; typedef scope-rollback group:
  **3 passed**. Diagnostic text in the negative scope tests is expected.

## Already solved / not copied

`trademomentumllc/claudes-c-compiler` `57d17369fc` fixes narrowing-cast truth
folding in the original compiler. Current LCCC already uses
`cast_integer_const` in the global CFG resolver and handles full-width integer
semantics. Its conservative local resolver falls back to that global resolver.
Importing the old fork's narrower integer-only implementation is not an upgrade.

`ChaseWNorton/claudes-c-compiler` `1827f0d994` removes a false i686 ECX clobber
for constant GEPs in the old allocator. Its absolute “101 bytes” result is not
transferable to the substantially changed current backend. No unmeasured
allocator patch is included on that evidence alone.

## Additional finding requiring a separate fix

GNU inferred declarations that reuse an **object** name still have a
pre-existing semantic/lowering binding gap, distinct from parser typedef
ambiguity. The deterministic diagnostic witness is:

```c
static int outer = 17;
int main(void) {
    __auto_type outer = &outer;
    return *outer != 17;
}
```

With `-O0 -Werror`, **both baseline and patched LCCC reject** this as an
`int *` initializer from `int **`; GCC 16.2 accepts and returns zero. See
[open-auto-object-binding.json](evidence/fork-network-2026-11-27/open-auto-object-binding.json).
The value-shaped variant (`static long long outer=17; __auto_type outer=outer;`)
also exposed a runtime problem, but the diagnostic witness avoids relying on an
uninitialized generated value as the reproducer.

The GNU name binds after its initializer. `SemanticAnalyzer::analyze_declaration`
infers the initial type before binding, then declares the symbol before its
later `analyze_initializer` walk. `Lowerer::lower_local_decl_inner` similarly
registers the local before lowering the initializer. Moving only parser names
cannot repair both semantic resolution and value binding. Address-taking,
volatile sources, const-local caches, aggregate values, function/array decay
and scope restoration need coordinated tests before changing those paths.

The passing inferred regression deliberately checks the **typedef-shaped**
initializer, a separate negative control for the parser adaptation. Do not
claim the complete inferred-declaration pipeline is fixed. Clang rejects these
GNU self-name forms, so GCC is the relevant extension oracle rather than a
two-compiler majority vote.

## Environment and evidence limits

- VM: two virtual Xeon CPUs, 3.8 GiB RAM; **not** an i7-14700KF.
- Active swap: 8 GiB, used during constrained compilation.
- Reviewed tag cache: `harness-cache-8`, tag object
  `d66c5ac881e2168790b8e228dd548f025ab83e59`, verified against the in-tree lock.
- Cached tools: GCC 16.2.0, Clang 23.1.5, GNU binutils banner
  `2.47.20260726`, mold 3.0.0, GDB 18.1.
- Rust was upgraded from cached 1.99.1 to stable 1.100.0 (official stable
  manifest dated 2026-11-12; all component SHA-256 values checked). Cargo,
  rustfmt and clippy were installed together.
- The first filtered lib-test compilation took 525.49 s / peak RSS
  3,679,400 KiB. Subsequent filtered groups reused the binary. Only **39**
  selected Rust tests ran; the other 9,789 tests were not claimed as validated.
- Cached multilib packages needed the matching Debian x32 dependencies;
  these were installed and `dpkg --audit` is clean. Valgrind 3.19.0 is available.
- Initial Intel requests failed, but an IPv4 retry succeeded. The existing
  rootless `icx_provision.py` then verified signed APT metadata/package digests
  and installed **ICX 2026.2.1-990**. The actual macro is **20260201**; its
  provisioning receipt and four direct functional checks are retained.
- Compiler Explorer's `cicxlatest` actually probed **20240000**. Its label is
  not an ICX-2026 identity! The final native measurements use the installed
  2026.2.1 compiler; the initial remote-channel probe is retained as evidence
  of the discrepancy, not mixed into the current native competition table.
- The latest LLVM release observed was **23.1.6**, newer than the reviewed CI
  cache. `restore_llvm_oracle_slice.py` installs a separately pinned Clang/LLD
  research slice plus x86-64 ASan/UBSan, without building/unpacking all LLVM.
  Six offline tests and real cold/warm compiler/header/sanitizer probes pass.
  The initial ad-hoc extraction missed LLVM's new triple-directory sanitizer
  layout; the durable script handles both layouts and cannot publish before
  link-and-run probes pass. It never overlays an unverified prefix.
- Current Clang 23.1.6 runs the three portable compiler fixtures and the CRC
  benchmark under ASan/UBSan. Native x86-64 **and** i686 constructor/printf
  smoke tests pass with **LCCC-ld, BFD 2.47, mold 3.0.0 and LLD 23.1.6**.
  The existing mold preference contract passes five tests: source mode uses
  `--no-default-features --features x86_64,i386,system-allocator -j2`; mold 3.x
  is Rust, so the old CMake target preset is no longer applicable.
- The GCC 16 cache lacks ASan/UBSan link inputs. The first source-sanitizer
  check therefore used explicitly labelled system GCC 12.2; the subsequent
  **Clang 23.1.6** sanitizer run passes too. This limitation is not hidden by
  calling the GCC 12 sanitizer an oracle-version upgrade.
- Optional current LLVM provisioning does **not** mutate the reviewed
  `harness-cache-8` lock or the exact CI Clang 23.1.5 pin. Publishing a refreshed
  cache and changing that CI contract is separate maintainer work, not an
  unreviewed tag update. No commits, pushes or release publications were made.
- No hardware-PMU or Raptor Lake performance claim is made. Correcting a wrong
  result is not a runtime speedup, and fewer static instructions alone would
  not establish one either. The VM advertises features beyond Raptor Lake;
  `-march=x86-64-v3` constrains emitted benchmark objects, not every libc/Intel
  runtime-dispatch choice. Those choices were not exhaustively traced, so the
  results do not claim a uniform whole-program SIMD ceiling or portability of
  the timing ratios to the i7-14700KF.

## Benchmark and measurement infrastructure completed

### A real new algorithm, not a renamed checksum scan

`tests/benchmark/programs/zlib_ng_crc32_combine.c` extracts zlib-ng 2.3.3's
GF(2) polynomial multiplication/exponentiation used for CRC concatenation.
The selector is the actual `archpkgbuilds` recipe at
`f705b69e16028cb0699fbb1b1e657ef09e6bca23`; original file hashes, alterations
and the Zlib license mapping are recorded. It self-checks the canonical CRC
vector, all **257 byte-buffer splits**, and **320 basis/large-length cases**
against a separate matrix algorithm. It is registered in the integrated corpus
with bounded Callgrind sizing; the canonical CI shard subset is unchanged.

Nine x86-64 compiler/optimization combinations (LCCC/GCC/Clang at O0/O2/O3),
two i686 builds, labelled GCC 12 ASan/UBSan and later Clang 23.1.6 ASan/UBSan
runs pass. Native ICX 2026.2.1 and **remote ICC Classic 2021.10** also produce
`zlib_ng_crc32_combine: f13e1784` at `PASSES=128`. The remote execution is a
functional check only, not a same-host timing reference.

### Oracle defects exposed and fixed during actual use

- The default whole-file survey could return success when one compiler failed
  but another emitted code. Requested compiler failures are now distinct from
  legitimate inlined/removed functions and make the invocation fail.
- `--local none --function f` invented a missing LCCC result. Only requested
  compilers are now represented, including in Markdown output.
- “Top wins” included losing rows; wins/gaps now have the appropriate signs.
  Arithmetic mean, geometric mean, best and worst ratios remain separately
  visible. Rows are deterministically ordered.
- **Unequal call scopes cannot be ranked.** CRC initially appeared as 55 LCCC
  instructions versus 55 GCC instructions, but LCCC had an outlined helper
  while GCC inlined it. Raw assembly/statistics remain visible; the row is now
  explicitly unranked and a strict comparison gate refuses it. Equal call
  counts still do not prove equal work.
- The derived survey sent `icx2026.2` to Compiler Explorer, which has no such
  ID. It now uses the existing native dispatch, retaining actual release,
  binary hash and provisioning metadata. Offline mode cannot silently start
  a provisioner; remote runtime mode rejects native aliases explicitly.
- A live freshness check found stale AArch64/RISC-V GCC 16.1 defaults. They now
  use 16.2, retaining explicit older aliases for bisection. The live audit passes.

The comparison contract has **12 passing offline tests**, including failing
baseline witnesses. The existing local-oracle suite passes **24/24**, including
its two real-ICX tests when enabled. The shared Godbolt cache self-test passes.

### Native ICX runtime support without handicapping optimization

The neutral-GCC-driver batched harness initially could not link Expat's ICX
object: `_intel_fast_memset` was unresolved. Disabling builtins/vectorization
would bias the oracle. `benchmark_native_batched.py` instead accepts explicit
`--runtime-library LABEL=ARCHIVE`: a self-contained static archive is hashed,
recorded in the experiment identity, applied only to that arm, and checked for
mutation before linking. Both measured/original link paths retain the selected
runtime; the common driver and object-renaming identity checks remain intact.
Thin archives and unknown arm labels are refused. The identity invalidates stale
resume/reuse records. Compiler/archive hashing is streamed to bound memory.

The ICX arm uses its signed-package `libirc.a`; no other arm acquires that
runtime. **17 harness tests and all 30 measured-object/output checks** pass.
The direct-test entry point was moved after all test classes so direct and
unittest-discovery invocation cover the same tests.

## Measured results — no new compiler-speedup claim

All native measurements below use **GCC 16.2.0, Clang 23.1.6 and ICX 2026.2.1**,
21 randomized paired rounds, CPU 0 affinity, an excluded warm-up round, and a
common calibrated batch count. Fork/exec/wait are excluded; entry page faults
and in-program setup/self-checks are included. The neutral driver remains GCC.
ICC is absent from the native table because no same-host ICC executable was
installed. Do not mix its remote executor time into these ratios.

**Patched and baseline LCCC generated byte-identical measured executables for
all six cases.** The harness explicitly shares samples for identical binaries
(`measured_as.before == lccc`). These are not two independent timing series and
not evidence of a new speedup. Five corresponding Callgrind A/B instruction
summaries are exactly equal as well.

### Bounded screening geometry

Point estimates are medians of paired time ratios, **GCC = 1.000; lower is
faster**. Common sizing flags are in `native-bounded.json`, not omitted from the
experiment identity.

| Kernel | LCCC | GCC | Clang 23.1.6 | ICC native | ICX 2026.2.1 |
|---|---:|---:|---:|---:|---:|
| zlib-ng Adler composition | 0.4366 | 1.0000 | 0.9790 | — | 0.9850 |
| gzip longest-match | 1.1202 | 1.0000 | 1.0234 | — | 0.9402 |
| Expat UTF-8 content | 1.0611 | 1.0000 | 1.0063 | — | 0.9202 |
| SQLite serial decoding | 1.2120 | 1.0000 | 1.0360 | — | 1.0165 |
| glibc dynamic-symbol hash | 1.0055 | 1.0000 | 1.0202 | — | 1.0232 |
| zlib-ng CRC composition | **7.8426** | 1.0000 | 0.5255 | — | 0.9137 |

LCCC/GCC descriptive aggregation across those six bounded entries:
**arithmetic mean 2.1130, geometric mean 1.3059, best 0.4366, worst 7.8426**.
Against the best measured non-LCCC arm per case (using the actual paired
samples), mean 3.3072 / geomean 1.4901, best 0.4451 / worst 14.8488.
These are workload-entry screens, not hot-kernel or application rankings.

**The large CRC outlier and apparent Adler win are scope warnings, not facts to
hide.** At very small PASSES, correctness/reference work dominates. For bounded
Adler, Callgrind attributes 13,007,634 of LCCC's 13,881,337 summary instructions
to the bytewise reference; the operator contributes only 551,605. Thus that
0.4366 row does not mean the composition operator is 2.3x faster.

### Default-size checksum controls

The default workloads were measured separately, without the bounded PASSES and
input-size overrides. Do not silently replace the bounded outlier with this
more flattering table or combine both scales into one aggregate.

| Default checksum workload | LCCC/GCC | Clang/GCC | ICX/GCC | LCCC versus best native oracle |
|---|---:|---:|---:|---:|
| Adler composition, 50,000,000 passes | 0.92125 | 0.92757 | 0.93513 | 0.99260 vs Clang |
| CRC composition, 262,144 passes | 0.99924 | 0.81687 | 0.80837 | **1.23078 vs ICX** |

For LCCC/GCC, mean 0.96025 / geomean 0.95945, best 0.92125 / worst 0.99924.
For paired LCCC/best-native ratios, mean 1.11169 / geomean 1.10530, best
0.99260 / worst 1.23078. The CRC LCCC/GCC bootstrap interval includes 1
([0.99612, 1.00055]); its native ICX gap remains the important candidate.
All intervals are exploratory VM intervals, not bare-metal guarantees.

### Counter accounting is checked, not assumed

At default CRC size Callgrind summaries report **1,173,263,475 LCCC** versus
**953,170,576 GCC** instructions. Cachegrind independently reports
**1,173,262,927** versus **953,170,028**: both tools give the **same exact
220,092,899 instruction-work delta**, despite a common 548-instruction startup
accounting difference. These are simulated guest-instruction counts, not uops,
cycles or physical cache/branch counters.

Callgrind's GCC cost-record `totals: Ir` differs from its `summary: Ir` by
2,351,183 instructions. Raw files and both headers are retained. Do not divide
per-function cost-record sums by the summary and present the resulting
percentages as exact; the cross-tool check corroborates the summary difference,
not every detail of attribution. No unexplained discrepancy is discarded.

### Controlled inlining spike, deliberately not shipped as a global heuristic

The actual inline trace first rejects `multmodp` at **13 blocks / 35 IR
instructions / 3 calls**. After simplification it becomes an **8-block /
33-instruction / 4-call** candidate; a helper call still remains in the final
operator. A source-level `always_inline` treatment on only this helper passes
all self-checks and 15 interleaved A/B runs: median 116.204 ms -> 113.177 ms,
**2.60%** lower, 14/15 paired wins. The edit, wrapper, compiler/source hashes,
commands and every sample are in `crc-inline-spike.json`.

This is **not an integrated compiler optimization** and does not explain the
whole 23% ICX gap. Global inlining-cap increases were not shipped: current
`inline.rs` documents real glibc/Linux losses from earlier broad relaxations.
The next prototype needs per-site loop/pressure evidence and held-out workloads,
not a benchmark-name special case or an indiscriminate threshold increase.

## Targeted validation inventory

| Scope | Result |
|---|---|
| Rust preprocessor / declaration / typedef-scope groups | 15 + 21 + 3 PASS; 10 new tests included |
| Library Clippy, Rust 1.100, fastbuild/O1/-j2, `-D warnings` | PASS; 60.65 s, peak RSS 3,561,756 KiB |
| Formatting of the three changed Rust files | PASS |
| Four new compiler fixtures through existing regression runner | 4 PASS, no A/B slot-layout failures; unrelated kernel boot gate explicitly skipped |
| x86-64 semantic matrices | all selected O0/O1/O2/O3/Os checks PASS |
| i686 LCCC/GCC matrix, four fixtures at O0/O2 | 16 PASS |
| Existing type-attribute/static-init and ternary gates | PASS, including their x86-64/i686 cases |
| Seeded frontend differential sweep | 32 mixed cases, O0/O2, GCC/Clang agreement |
| Fork inventory / LLVM slice / comparison / batched harness tests | 11 / 6 / 12 / 17 PASS |
| Oracle architecture / codegen / benchmark harness tests | 12 / 14 / 20 PASS |
| Local oracle tests with real ICX enabled | 24 PASS, no skips |
| mold preference / parallel CI-layout contracts | 5 / 15 PASS |
| CI invocation/configuration parity | PASS, 629 commands / 51 invocation contracts; **not execution of CI** |
| Native four-linker smoke | 8 PASS (four linkers x two x86 targets) |
| Full `ci_local.sh` / full torture / kernel / package-wide test suites | **NOT RUN** under the surgical-validation instruction |

Some unit suites deliberately print rejected-case diagnostics. Their passing
assertions verify refusal; they are not hidden production failures. The five
inaccessible fork records and the separately documented GNU object-binding
witness remain explicit coverage/defect findings, not “all green” claims.

## Reproduction and recovery

Use [the evidence index](evidence/fork-network-2026-11-27/README.md) for raw
samples, binary/source identities, archive/package hashes, counters and assembly.
Keep bulky sources, build products, swap and toolchains on the largest
**disk-backed** filesystem, outside the Arena publication directory.

```sh
# Existing reviewed cache, then an explicit current LLVM research slice.
python3 scripts/restore_agent_toolchain_cache.py \
  --prefix /large/drive/agent-tools --only rust,gcc,clang,binutils,mold,gdb
python3 scripts/restore_llvm_oracle_slice.py --prefix /large/drive/llvm-23.1.6
python3 scripts/icx_provision.py /large/drive/icx-2026.2
# Use the returned ICX path; the native name must not be sent to CE.
export LCCC_ICX_BIN=/large/drive/icx-2026.2/opt/intel/oneapi/compiler/2026.2/bin/icx
LCCC_BUILD_JOBS=2 LCCC_SWAPFILE=/large/drive/lccc.swap scripts/build_lccc_fast.sh
python3 scripts/godbolt_codegen_compare.py \
  tests/benchmark/programs/zlib_ng_adler32_combine.c --function zng_combine \
  --oracles gcc16.2,clang,icc,icx2026.2 --flags='-O2 -march=x86-64-v3'
# For the native batched ICX arm, retain the compiler's runtime:
# --runtime-library icx=/large/drive/icx-2026.2/opt/intel/oneapi/compiler/2026.2/lib/libirc.a
```

Select/update stable Rust separately as the repository manifest requests;
restoring the historical cache alone yields Rust 1.99.1, not the 1.100.0 used
for final validation here. Keep the exact CI Clang pin separate from the
explicit 23.1.6 research arm. The existing linker provisioning already honors
mold's x86_64+i386 source feature selection; its tests verify cache/source
switching rather than treating a version string as provenance.

Arena snapshots use `MS178_PUBLISH_DIR=/home/user/workspace`, a bounded history
under `snapshots/`, and `MS178_UNGATED=1` to state the targeted-only validation
honestly. After a wipe, restore the recorded baseline, check/apply the cumulative
patch, re-provision tools and rerun only affected gates. Do not manufacture a
full-CI stamp. Local workspace snapshots are not a guarantee against a complete
platform-wide loss; the published patch/checksum are the exportable artifacts.

## Remaining audit work and measurement discipline

The branch census is an inventory, not a line-by-line correctness proof of all
public forks or of LCCC. Unavailable/private/deleted forks and non-advertised
refs remain outside visibility. Broader adoption needs a concrete current-LCCC
reproducer and provenance/license review before any foreign code is run.

| Priority | Remaining item | Evidence / expected benefit | Admission condition |
|---|---|---|---|
| P0 correctness | GNU inferred **object** binding | Deterministic before/after rejection of the valid GCC witness above; semantic and lowering namespaces bind too early | Fix both phases; test address-taking, scalars, volatile/const caches, aggregates, function/array decay and nested-scope restoration. Do not merely silence the diagnostic. |
| P1 measurement | Explicit current-ICX policy | CE `latest` probes 2024; native 2026.2.1 is now available and correctly routed | Keep macro/binary/package identities; use `icx2026.2` or an explicit local reference for 2026 claims. Never silently provision in offline mode. |
| P1 codegen | CRC loop/site cost and backend dependency shape | Default-size LCCC/best-native median 1.23078; forced-inline hint improves only 2.60% in its separate screen | Add per-call-site missed-inline reasons, inspect remaining dependencies/register pressure, test a narrow cost-model prototype on held-out real workloads and Raptor Lake. No blanket cap increase. |
| P1 codegen | SQLite serial decode, gzip matching, Expat scanning | Bounded native LCCC/best paired medians 1.2120, 1.1792, 1.1523 | First retain input geometry and inspect the generated hot regions; graduate to full package workloads before claiming application gains. |
| P2 fork research | Pressure hoisting / return duplication / anti-dependence splitting | Promising designs, but concrete ordering/span/ABI integration blockers are recorded in the admission audit | Repair invariants in isolated prototypes, run dominance/source-span tests and measure register pressure/code growth before integration. |
| P2 toolchain | Refresh the reviewed agent cache | Stable Rust 1.100 and LLVM 23.1.6 are newer than parts of cache-8; GCC16 sanitizer inputs are missing | Publish only after license/provenance review and authorization; update cache lock and CI pins together. Do not commit fictitious digests or redistribute Intel runtime binaries in this patch. |
| P2 audit | Remaining fork semantics / unavailable heads | Complete visible-head inventory is not complete semantic review; five access gaps remain | Reuse immutable tip/source signatures to audit only new/different code; obtain missing source/access rather than classifying unavailable forks as safe. |
| P2 tooling | Wider survey artifact identity | This session's program basenames are unique; generic multi-directory callers can still need collision-resistant artifact naming and complete source identity | Extend tests before broader corpus use; keep the existing single-source provenance-aware Godbolt comparison for evidence capture. |

For future performance experiments, preserve binary/source/flag identities;
use the existing deterministic Callgrind counts as instruction-work evidence,
not cycle or cache-PMU evidence. Randomize/interleave native A/B repetitions,
verify outputs before timing, report per-workload regressions as well as
geometric/arithmetic means, and validate any Raptor Lake claim on that machine.
