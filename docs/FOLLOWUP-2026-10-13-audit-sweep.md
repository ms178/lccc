# Follow-up — 2026-10-13 · AUDIT-2026-10-13 sweep (session 2)

Baseline: `7e94441` (upstream `ms178/lccc` main at session start).
Deliverable: `ms178-1.patch` (snapshots `S01`, `S02`, … in the session ledger).
Scope of this document: what the audit's item list asked for, what is now
**fixed and gated**, what is **verified-open** (with the entry point a future
session should start from), and the standing environment/harness rules this
session had to relearn.

---

## 1. Accomplished in this session

### AUD-VABI-1 — a 16-byte vector crosses a compiler boundary in ONE `%xmm` (FIXED, cross-compiler verified)

*Symptom.* `<4 x i32>` (a `vector_size(16)` C vector) returned from a
GCC-compiled callee to an lccc caller gave `16 0 65536 0`, and the mirrored
combination (lccc callee, GCC caller) gave `1 2 3 4` — both rc=0, both silently
wrong. `{double,double}` (the two-`xmm` struct control) was already correct in
all four combinations, which is why same-compiler testing could never see it.

*Cause.* Vectors travelled the by-value-aggregate path with **empty**
`struct_arg_classes`, i.e. they were classified INTEGER/GP. The psABI
classifies `vector_size(16)` as SSE + SSEUP: the whole 16 bytes live in ONE
`%xmm` (GCC oracle asm: `movdqa`), while the struct control legitimately uses
two registers.

*Fix.* `CType::sysv_vector_carrier()` returns `Some(([Sse], false))` for
≤8-byte vectors, `Some(([Sse, Sse], true))` for 16-byte vectors — reusing the
existing "one 16-byte %xmm" carrier that the `_Float128` path already had
(`is_f128_sse`, `ParamClass::F128SseReg`, `movdqu` in/out), so no new backend
path was needed — and `None` for anything else (including 32-byte, which needs
an `-mavx` decision). Wired into: return classification + `ret_is_f128_sse`,
both parameter paths (declaration and definition), the argument side, and the
three **indirect-call** arms, which have no `FuncSig` to read and hardcoded
`ret_is_f128_sse: false` — those now derive the carrier from the callee
expression's own function type, so `pick_impl(0)(a,b,c)` reads `%xmm0` where
the callee wrote it.

*Guard.* `target_elf_machine() == 62`. The flag is target-relative (AArch64 Q
register, RISC-V GP pair) — **do not generalise it** without that work.

*Gate.* `tests/regression/check_vector_abi_boundary.sh` (+ fixture
`tests/regression/vector_abi_boundary/vector_abi.c`, split into caller/callee
TUs with `CALLEE_ONLY`) links each caller/callee combination against GCC at
`-O0` and `-O2`, asserts the four combinations' output, and asserts the
**arrival registers** (`%xmm1` for a `v2df` pair, `%xmm3` for the
`{double,double}` control) so the two conventions cannot silently merge.
Mutation-verified: neutering the carrier helper fails the gate.

### AUD-CPU-3 (remainder) — VNNI and GFNI are gated (FIXED for these families)

The audit's live SIGILL case was AVX-VNNI: `_mm_dpbusd_epi32` compiled with
`-march=x86-64` emitted `vpdpbusd %ymm1, %ymm0, %ymm0` — an Alder-Lake-era
instruction inside a v1 object. Closing it needed three feature bits, not one:
AVX-VNNI (`vpdpbusd`/`vpdpwssd`), AVX-VNNI-INT8 (`vpdpbssd`/`vpdpbsud`/
`vpdpbuud`) and AVX-VNNI-INT16 (`vpdpwusd`/`vpdpwuud`) are three CPUID bits,
three GCC flags and three separate promises, so `X86Feature` goes 24 → 27 and
the 28 builtins in that family each get a row.

The permission is the same conjunction VAES/VPCLMULQDQ uses — 256-bit vector
class (`-mavx2`/AVX-512) AND the extension's own flag — because that is what
the emitter needs and what GCC's header guard implements. Verified against GCC
12.2 row by row: reject on `-march=x86-64`/`-v3`, on `-mavx2`, on
`-mavx512vnni` without VL and on `-mno-avx`/`-mno-avx2`; accept on
`-mavxvnni`, on `-mavx512vnni -mavx512vl` and on `-march=alderlake` — lccc
agrees with the oracle on every one of those. One deviation is asserted in the
gate rather than compared: GCC's `-mavxvnni` also implies AVX2, so
`-march=x86-64 -mavxvnni` is accepted there and refused here (the user asked
for a v1 baseline and the 256-bit class is what the backend would need).
INT8/INT16 have no oracle on this toolchain — GCC 12.2 does not know either
flag and declares none of those intrinsics — so the gate says so and pins the
lccc-side contract instead.

Landed with it: `-mno-avxvnni` (the enabling flag existed, the denial arm did
not, so the denial was silently ignored), the intrinsic permission rows for all
three names, the CPU-support gate extended to 27 probed features
(Alder/Raptor Lake bring AVX-VNNI; Arrow Lake/Sierra Forest add INT8;
`-march=arrowlake-s`/Lunar Lake add INT16), and 27 new intrinsic-gate rows.

GFNI went in the same way (28 features now): `gf2p8mulb` is **legacy-encoded**,
so its permission is the flag alone — `-mgfni -mno-avx` accepts in both
compilers, measured — and before the row existed lccc emitted
`gf2p8mulb %xmm1, %xmm0` for `-march=x86-64` with no diagnostic. lccc now
agrees with GCC 12.2 on every one of the eight oracle rows for that family,
including the `-maes` and `-mno-avx` corners.

**Found while doing it: AUD-CPU-4** (see §2) — the 32-bit target emits 256-bit
VEX code with no permission at all, in the *vector-op* path, while the
intrinsic table refuses the same class of instruction there. Latched by the
same gate rather than quietly fixed inside this change.

### AUD-RATCHET-1 — the audit census is measurable and enforced (DONE)

`scripts/audit_census.py` + `scripts/audit_census_baseline.json`:
panic-class sites (and the hot subset in `frontend/` + `ir/lowering/` +
`driver/`), `unsafe` sites, unannotated `#[ignore]`, functions over
300/500/1000 lines, `CCC_*`/`LCCC_*` literals and how many **no** markdown file
names. `--check` fails the build when any count grows; `--update` re-baselines
(it warns when a metric rises); `--selftest` pins the tokenizer (line comments,
nested block comments, raw strings, `#[cfg(test)]` truncation, line-number
preservation); `--top-fns N` and `--list-undocumented-knobs` are the worklist
views.

*Calibration (why the numbers are trustworthy).* Running `--top-fns` on this
tree reproduces the audit's published worst-offender table **exactly**:
`emit_intrinsic_impl` 5,839 @ `x86/codegen/intrinsics.rs:1713`;
`emit_shared.rs:179` 4,135; `emit_exec.rs:115` 4,120; `emit_script.rs:1009`
3,094; `encode_mnemonic` 3,056 @ `x86/assembler/encoder/mod.rs:3371`;
`allocate_registers` 2,669 @ `regalloc.rs:3111`; `prologue.rs:289` 2,446;
`slp_vectorizer.rs:2567` 2,235. The metric definitions in the docstring are
therefore the audit's own, not an approximation of them.

Baseline at `1f0261a`: `panic_sites` 889 (109 hot), `unsafe_sites` 27,
`ignored_tests` 1, `fns_over_300/500/1000` 192/76/23, `knob_literals` 498,
`knobs_undocumented` 292.

Wired into `scripts/ci_local.sh` (two fast gates) **and**
`.github/workflows/ci.yml`; `scripts/check_ci_gate_parity.py` is green (188
commands), which is the mechanism that keeps the two mirrors from drifting.

### AUD-HYGIENE-1 — tracked files that should not be in the repository (DONE)

* The tracked Claude Code permissions file (it granted every clone blanket
  shell auto-approval), the leaked snapshot anchor at the repository root, and
  the tracked scratch directory that `.gitignore` already listed
  (assembly + two ELF executables referenced by no script) are gone, with
  `.gitignore` entries so they cannot come back.
* The loose `tests/*.{c,s,sh}` became real gates — and this is where the audit's
  "move with a gate or delete" turned up value:
  * `tests/regression/check_compat_suite.sh` (was `tests/compat_test.sh`, 20
    programs, previously unreferenced): now honours `$CCC`, uses a private
    `mktemp -d` scratch (it used fixed `/tmp/lccc_test*` paths, so two runs
    clobbered each other), prints the compiler's own stderr on failure, and
    **exits non-zero** (it always exited 0 before, so it could not fail
    anything). 20/20 vs GCC in ~2 s.
  * `tests/regression/check_ivsr_reproducers.sh` + the probe sources under
    `tests/regression/fixtures/ivsr/`: metamorphic rather than shape-pinned —
    the same programs must compute identical numbers at `-O0..-O3` with
    `CCC_NO_IVSR` on and off, differentially against GCC. Pinning today's
    instruction selection would fight every future improvement; pinning the
    *equivalence* protects the property that matters.
  * `fib_rec2iter.c` / `verify_fib_perf.sh` moved next to the benchmark
    workflow's new path (`.github/workflows/bench.yml` updated in lockstep).
* Stale pre-fix `.s` snapshots deleted (they were records of a state, not
  tests).
* `artifacts/repros/` (97 programs, the directory's own scar tissue) replayed
  by `scripts/check_repro_replay.py`: **none of them still reproduce**. Getting
  there needed the ISA to be pinned on both compilers, which is the finding
  worth keeping — see the box below. The three `csmith` programs that
  `#include "csmith.h"` (a header the generator emits and this repository does
  not vendor) are *declared* rather than skipped, and the declaration is
  checked in both directions so it cannot rot into a skip list. The bracketed
  file names (`[CCC_RA_GLOBAL_LOCATION=1]`) now use the shell-safe
  `__env_CCC_RA_GLOBAL_LOCATION-1` form, and `artifacts/repros/README.md`
  documents the naming contract the gate parses.

> **Reading a differential result: pin the ISA on both sides.** An unpinned
> replay of `artifacts/repros/` reports 29 failures; with `-march=x86-64` handed
> to both compilers, 94 of 97 agree (the other three are the header-dependent
> csmith files). The gap is not codegen: lccc's documented project baseline is
> `x86-64-v3`, GCC's default is plain `x86-64`, and lccc's backend **contracts
> `a*b+c` into FMA at every level including `-O0`** (measured: 10 `vfmadd`/
> `vfmsub` sites in an `-O0 -march=x86-64-v3` build) while GCC starts
> contracting at `-O2`. Contraction is implementation-defined in C, so this is
> legal — but it means `-O0` numerics differ from GCC unless the comparison is
> pinned. The gate therefore runs two pinned legs (`-march=x86-64` and
> `-march=x86-64-v3 -mno-fma`); a `-ffp-contract=` flag is the obvious knob if
> GCC-identical `-O0` numerics are ever required.

### AUD-SILENT-ZERO-1 — the remainder (FIXED, two layers + the ICEs)

Previously the non-x86 emitters' "x86-only" arm stored 16 zero bytes when the
destination was a known slot and **nothing at all** otherwise, so an x86
intrinsic compiled for AArch64/RISC-V produced a wrong value with rc=0. The
first half (previous session) replaced that with the target's trap at runtime.
This session added the compile-time contract the audit asked for, plus removal
of the remaining ICEs:

* Lowering (`check_x86_intrinsic_feature`, which is reached **only** from the
  arm that dispatches the x86 builtin families) now rejects any such call on a
  target that is not x86-64/i686:

  ```
  x86_only.c:4:69: error: `__builtin_ia32_aesenc128` is an x86-only intrinsic
  and is not available on aarch64 (aes instruction-set extension, -maes)
  ```

  No per-op table is needed to decide *that* it is unavailable — only to name
  the extension in the hint. `#ifdef __x86_64__`-guarded code still compiles:
  the diagnostic fires only when the call is actually lowered.
* Eight compiler ICEs replaced by the target's own trap (`emit_unreachable` →
  `brk #0` / `ebreak` / `ud2`), each with its invariant written down: five
  AArch64 arms (4-wide/AVX group, 256-bit group, three "dest is not
  slot-homed" f128 sign-bit paths), one RISC-V vector arm, one RISC-V f128
  path, one i686 vector arm. The f128 invariant is structural —
  `track_f128_self` registers a value whose readers resolve it through
  `f128_resolve_slot_addr`, so a register home would be invisible to its own
  consumers — and was checked to hold across `-O0..-O3` on the f128 intrinsic
  corpus (`__builtin_fabsf128`, `__builtin_copysignf128`, unary minus, 24 live
  f128 locals in one frame, nested intrinsic calls) before relying on it.
* Gate `tests/regression/check_cross_target_intrinsic_trap.sh` rewritten:
  (1) nine intrinsic **families** (`aesenc` 128, `pclmulqdq` 128, `crc32qi`,
  VAES-256, VPCLMULQDQ-256, `_mm256_add_ps`, `_mm256_mul_pd`,
  `_mm256_broadcastsi128_si256`, and the no-operand `_mm256_zeroupper` the
  AArch64 catch-all used to swallow) must each produce their own
  "not available on <arch>" diagnostic at `-O0` and `-O2` on AArch64 and
  RISC-V — nine families, nine independent rejections, counted;
  (2) portable builtins (`fixture .../portable_only.c`) still lower for all
  four targets; (3) x86 targets that *have* the extension still emit `aesenc`;
  (4) the safety net's trap still assembles and encodes
  (`0xd4200000` / `0x00100073`, read out of the ELF section table).

### Latent defect found by the full gate run (FIXED)

`backend::x86::codegen::peephole::passes::whitespace_invariance` collected its
corpus from `tests/` **and `work/`** — a directory that `.gitignore` lists.
Removing `work/` (AUD-HYGIENE-1) dropped the corpus below the test's vacuity
threshold, which is how the dependency surfaced: an invariant test whose inputs
can be deleted by a cleanup is not evidence. The second root is now
`engineering/evidence/godbolt/corpus` (committed GCC/Clang/ICX output — also the
more honest corpus, since asm-diff feeds the peephole foreign assembly).

### AUD-ICE-1 — the acceptance gate, and the arity half of the fix (GATED + PARTIAL)

*Measured first.* The audit's own reproduction (`__builtin_shuffle` with an
`int` mask) was already fixed; what the gate found underneath it was a whole
class: ill-typed calls to *any* intrinsic were passed to the backend, which
panics on operand indexing (`ccc: internal error: emit_sse_binary_128:
malformed intrinsic addps (0 args)`) — 650 of 624 × 2 names × optimisation
levels at the time of measurement, and separately 380 that compiled silently.

*The gate.* 624 names × 4 shapes × 2 levels, table-driven from
`src/frontend/sema/builtins.rs` so a builtin added tomorrow is covered
tomorrow. Compiles are full `-S` compiles (`-fsyntax-only` never reaches the
backend, and the backend is where the panics live). Both the ICE list and the
silent list are ratcheted against
`tests/regression/ill_typed_builtins_baseline.json`; the ratchet logic has a
self-test, and the first run of *that* self-test is what proved the gate can
fail (a stale baseline entry is a note, a missing one is a hard red).

*The arity fix.* `src/frontend/sema/intrinsic_arity.rs` (generated, 303 rows)
gives the front end the argument count of a builtin under its own name, and
the `builtins::is_builtin` arm of `src/frontend/sema/analysis.rs` reports
`too few`/`too many arguments to function '…' (expected N, have M)` when the
name is not declared in scope — GCC's wording, GCC's verdict. Enforcing it
only for undeclared names is what keeps a header-included translation unit
from being told twice.

*Why the headers.* An intrinsic is only a prototype when the translation unit
included the header that declares it, and the `__builtin_ia32_*` primitives
are never declared anywhere — so the count is derived from the headers
themselves and checked in, with `--check` in CI to stop the table from
rotting. A name whose header uses disagree, or that no header mentions, is
left unconstrained: an unverifiable guess would start rejecting valid code.

### The hosted Test Suite red that was ours — `fib_rec2iter` (FIXED)

`7b0ea71` added a `fib_rec2iter` corpus test: it evaluates `fib(90)`, which the
rec2iter pass makes instant and which **GCC cannot do at all** (2^90 calls).
The corpus runner therefore killed the reference compiler's leg at the
per-test timeout and reported `compare:fail (gcc rc=-9)` — a test that can
never pass, which is why the hosted `Test Suite` job was red while every local
run of the same suite showed "only the known host-limited failures".

*Lesson for the harness rules: a red that the local host cannot reproduce
(gcc's leg here is a timeout, not a rejection) is invisible in a per-test diff
and only shows up as `compare:fail` — check `rc=-9` before assuming an lccc
regression.*

This session first fixed it by declaring the test `LCCC_NO_COMPARE`. Upstream's
PR #815 then settled the same problem at the root, and **that resolution is the
one that survives the merge**: the program moved out of the corpus glob into
`tests/regression/fixtures/rec2iter/`, where `scripts/verify_fib_perf.sh` and
the `rec2iter` gate in `scripts/ci_local.sh` run it as a self-check rather than
a differential — a fixture the corpus cannot pick up cannot produce a phantom
`compare:fail` on any host. The `.env` workaround is therefore gone.

### Merging upstream PR #815 into this branch (INTEGRATION)

Both sessions worked the same audit list, so the merge hit fourteen files with
real overlap. The rule applied throughout: keep whichever side is *stronger*,
and never keep two copies of the same thing.

* **`__builtin_cpu_supports` — upstream's runtime probe wins** (AUD-CPU-1).
  This branch had folded the builtin to a compile-time constant from the
  invocation's permission; upstream probes CPUID at run time through a
  libgcc-compatible `__cpu_model` (`src/common/cpu_probe.rs`, lazy per-function
  init, GCC's own error wording, constant-context diagnosis). Upstream measured
  the competition before choosing: GCC 16.2, Clang 23.1 and ICX all compile
  `!!__builtin_cpu_supports("sse2")` into a load from `__cpu_model` *even when
  the effective target enables `sse2`*, because the question is about the CPU
  that executes the program. The fold answers a different question: a
  `-march=x86-64-v3` binary that folded `"avx2"` to `1` claims AVX2 on hardware
  that lacks it — the one answer none of the three gives — and it is also what
  runtime-dispatch code needs the other way round, since a `-march=x86-64`
  build must still be able to take an AVX2 path where AVX2 exists.
  Consequently the audit's named gate
  `check_cpu_supports_fold.sh` and the `.env` file that suppressed the GCC
  comparison are **deleted**, and `check_cpu_supports_runtime.sh` (upstream's,
  differential in every row) is the gate. What this branch adds on top of
  upstream's implementation is the audit's third requirement: a name from the
  x86 feature set is now *diagnosed* on a non-x86 target instead of silently
  folding to 0, while names from other architectures' sets keep the
  conservative 0 (upstream's reason stands: those sets are GCC's per-architecture
  lists, which LCCC does not model, and rejecting them would turn accepted GCC
  code into a compile error).
* **The intrinsic permission gate stays ours** (AUD-CPU-3): upstream rejects
  x86 intrinsics on foreign targets but never asks whether the *invocation*
  granted the extension. `x86_features::effective()` remains the model for
  that question, and the non-x86 half of the check now defers to
  `lower_x86_intrinsic`'s rejection so one bad call produces one diagnostic,
  not two.
* **Temp files, debug dumps and the peephole corpus stay ours** (AUD-TMP-1,
  AUD-HYGIENE-1): the private 0700 directory with the ownership probe, the
  symlink-safe debug dump, and the whitespace-invariance corpus rooted in
  committed assembly only (`tests/` + `engineering/evidence/godbolt/corpus`)
  are stricter than upstream's equivalents, and `check_temp_symlink.sh` (182
  rows) is the gate for the first of them. Upstream's call sites that expected
  a fallible constructor were adapted to the infallible one.
* **`-o -` / `-M -o -` stay ours** (AUD-DRV-1): every text producer funnels
  through the single `write_text_output`, and `check_o_stdout.sh` pins it.
* **`-o -` has one guard and one gate after the merge.** Both sides added the
  same refusal on different code paths (this branch in
  `Pipeline::validate_output_destination`, upstream inline in `run()`), and
  both sides had a gate asserting *its own* message — so the tree carried two
  rejection messages for `-c -o -` and upstream's gate failed on ours (this
  branch's guard fires first). The validator now also refuses a link
  (`Cannot write an executable to standard output ('-o -')`), upstream's inline
  check is gone, and `tests/regression/check_o_stdout.sh` is the single gate:
  its rows are the union of both (upstream's pipe-shaped streaming rows,
  `-dM`, `-MF -` and the real-path negative control were folded in).
* **Re-anchoring onto a newer main broke the snapshot's proof, not the
  bundle.** `scripts/session_bundle.py` proved each bundle in an isolated
  clone that fetched BASE with `--depth=1`. A range that *merges* upstream
  records the old fork point as a second prerequisite beside BASE, so the
  proof rejected a bundle that restores perfectly (`Repository lacks these
  prerequisite commits: 7e94441…`), and `scripts/lccc_recover.py` would have
  done the same to the delivered artifact on the user's side. Both now retry
  against BASE **and its ancestry** (the receipt records `base_fetch`), and
  `tests/corpus/test_recovery_contracts.py` has a miniature merged-range repo
  that proves the bundle, restores it, and pins the offline answer. Without the
  fix the same test reproduces the reported error; the recovery half fails
  identically on the pre-fix tool.
* **`ci_local.sh`'s ill-typed-builtin gate passed an undefined `$REPO`** to the
  checker. Under `set -u` that killed the whole local suite mid-run — the
  reason no `--fast` stamp could ever be obtained on this branch — while the
  hosted CI step (which spells `LCCC_REPO=.`) stayed green. It is `LCCC_REPO=.`
  now, the same value CI uses; the script `cd`s to the repo root first.
* **`__builtin_shuffle*` diagnostics take upstream's wording**: both sides
  fixed the panic (AUD-ICE-1/AUD-DIAG-1), upstream's messages are GCC's own
  ("number of elements …", "must be an integer vector", "invalid element
  index"), and the gate now asserts those fragments.
* **Layout follows upstream** where it moved files: the IVSR probes are
  `tests/regression/fixtures/ivsr/`, the rec2iter program is
  `tests/regression/fixtures/rec2iter/`, `verify_fib_perf.sh` lives in
  `scripts/`, and the gate that consumed the old paths was re-pointed.
* **`artifacts/repros/` keeps this branch's `__env_` names**: the replay gate
  parses the environment out of the file name (`ENV_RE`), and upstream's
  `_default_` / `_CCC_RA_GLOBAL_LOCATION-1_` spellings do not match it — a name
  the gate cannot parse is a hard failure, so the enforced convention wins.
* **The census baseline moves by exactly what upstream brought in**: the
  merged tree measures `panic_sites` 889 → 890 (+1, the
  `expect("canonical_name returns table entries")` invariant in upstream's
  `src/common/isa_features.rs` — unreachable, since the name comes from the
  same table) and `unsafe_sites` 27 → 29 (+2, the `Self::cpuid` /
  `Self::cpuid_count` wrappers, `unsafe` for toolchains older than 1.97 with
  `#[allow(unused_unsafe)]` for the ones where `__cpuid` is already safe).
  Re-baselined in the merge commit that adds them, which is what the ratchet
  asks for; every other metric is unchanged.
* **Upstream's `check_cpu_supports_runtime.sh` gained two rows** for the audit's
  third requirement: an x86 feature name on aarch64/riscv64 is now expected to
  be *diagnosed* (upstream asserted the opposite — a silent 0), and a name from
  another architecture's set (`crypto`) is still expected to fold to the
  conservative 0, so upstream's reason survives the change.
* **Upstream's second replay gate is gone** in favour of
  `scripts/check_repro_replay.py` (both are described in `scripts/README.md`):
  one directory (`artifacts/repros/`) has one naming contract, and only one of
  the two gates can parse this repository's file names (`__env_<K>-<V>`), so
  keeping both meant a red CI from names the other gate could not read. Upstream's `audit-census` duplicate registrations
  are gone for the same reason.

* **`audit_census.py`, its baseline, and `check_ill_typed_builtins.py` stay
  ours**: the pair is one implementation plus one measured baseline (they must
  not be mixed), and the ICE gate is the one whose ratchet was measured
  (650 → 174 ICE, 380 → 294 silent).

### AUD-POS-1 — every diagnostic after a `#include` was one line high (FIXED)

*Symptom.* `printf '#include <stddef.h>\nint bad = ;\n'` compiled with LCCC
blamed `d.c:3:11` for the syntax error on physical line 2. GCC blames 2:11.
Any file that includes a header — i.e. every real translation unit — reported
every subsequent position one line too high.

*Cause.* `preprocess_source` splices an include's text into the output followed
by a return-to-parent line marker (`# N "file" 2`). That marker already names
the parent's **next** line, so the directive's own line is accounted for; the
"preserve line numbering" block at the end of the directive path pushed one
more `\n` anyway. The phantom blank line sat between the marker and the next
parent line, and everything the lexer mapped afterwards was off by one. The
shift is exactly +1 regardless of how many includes a file has, because each
later marker re-anchors the parent file's numbering — which is why two
includes did not produce +2 and why the bug looked like a rounding error.

*Fix.* `emitted_include_content` is set when the directive produced include
content, and the line-preservation newline is skipped in that case (the
`pp_output_line` accounting for macro-expansion diagnostics is deliberately
left alone — it counts the newlines actually written).

*Verified.* `tests/regression/check_diagnostic_positions.sh` compiles twelve
shapes with both compilers and requires the *same* `line:col`: error after an
include, include on line 2, two includes, local include, nested include, error
inside the header, no include at all, error six lines below an include, past a
`#define` and past a `#pragma`, the missing-header case, and `__LINE__` after
an include. Each row is checked in three ways — LCCC agrees with GCC, the row
actually produced a diagnostic, and the position is the one the fixture
documents — so the gate cannot pass by both compilers staying silent. Measured on the same host, the binary built
from the parent commit fails 8 of the 12 rows -- exactly the eight whose fixture puts the
error after an include -- and the fixed binary passes all 12, so the gate is shown to be
able to fail before it is trusted to pass.

---

## 2. Verified-open items (start here)

Ordered by value ÷ cost, with the entry point each one needs.

1. **AUD-CPU-3 remainder — with VAES/VPCLMULQDQ, the three VNNI groups and
   GFNI landed, the table covers every extension the frontend currently
   exposes; what is left is the AVX-512 layer (512-bit rows and a 512-bit class
   notion) and SHA, which has no builtins to gate yet.** `x86_intrinsic_required_feature` (`src/ir/lowering/expr_builtins.rs`)
   covers CRC32, the six 128-bit AES ops and `pclmulqdq128`; the audit names
   256-bit VAES/VPCLMULQDQ, `X86Crc32Load*`, SHA/GFNI/AVX-512 as missing, and
   the family sweep above now proves that at least VAES-256/VPCLMULQDQ-256
   reach lowering *without* a permission check on x86. Method that keeps it
   honest: `x86_features::X86Feature` has no `Sha`/`Gfni`/`Avx512*` variants
   yet (add them), the source-name → `BuiltinIntrinsic` mapping is a literal
   `m.insert("name", …)` table in `src/frontend/sema/builtins.rs` (624 rows,
   mechanically extractable), and the verdict for each row can be
   oracle-checked against `gcc -march=x86-64 -S` (error ⇒ the row needs a
   feature), exactly the way `check_intrinsic_feature_gate.sh` does it today.
   The AVX-VNNI half of this entry is now **done** (three bits, 28 rows, GCC
   oracle rows, and the intrinsic permission rows); the oracle rules that
   were measured for it are kept here because the same method is what the
   remaining families need, and because they are the record behind the rows
   (`-march=raptorlake` is unknown to GCC 12; use `-march=alderlake` for a CPU
   profile):

   ```
   _mm_dpbusd_epi32 / _mm256_dpbusd_epi32   (AVX-VNNI)
       -mavxvnni                -> accept  (128-bit AND 256-bit; VEX, not AVX2)
       -mavx2                   -> reject
       -mavx512vnni             -> reject  (128-bit forms need VL)
       -mavx512vnni -mavx512vl  -> accept
       -march=alderlake         -> accept

   _mm_gf2p8mul_epi8 / _mm256_gf2p8mul_epi8  (GFNI)
       -mgfni                   -> reject  (GFNI has no legacy encoding)
       -mgfni -mavx             -> accept
       -march=alderlake         -> accept
   ```

   That hole is closed (measured: `-march=x86-64` now diagnoses
   `_mm_dpbusd_epi32` and names `-mavxvnni`). What remains in this entry:

   * **SHA** — no `X86Sha*` builtin exists and `-msha` is not accepted, so
     there is nothing to gate: a dispatcher asking
     `__builtin_cpu_supports("sha")` gets 0, which is the safe answer, and the
     name stays in the untracked list with a test that keeps it there. Adding
     SHA means adding codegen first.
   * **AVX-512** — the `-mavx512*` flags exist in the CLI and the 512-bit
     builtins exist, but no row gates them; the fold has no 512-bit class
     notion (that is also what AUD-CPU-4 is about, one width down).
   * **`X86Crc32Load*`** — checked and not applicable as written: the CRC32
     family is exactly four builtins (`X86Crc32_8/16/32/64`), and the load
     forms are the *same* builtins with a memory operand, so the existing rows
     cover them.
   * **The 256-bit GFNI forms** — `_mm256_gf2p8mul_epi8` is declared by GCC but
     not by lccc's bundled headers (implicit-declaration warning), so they are
     unreachable from source until the headers grow them; that is a header
     change with an ABI question (32-byte vector argument), not a table row.
2. **AUD-ICE-1 remainder — the gate exists, the arity half is fixed, the
   type-class half is measured.** The acceptance gate is
   `scripts/check_ill_typed_builtins.py`: it reads the compiler's own name
   table (624 intrinsic names), calls every one of them through four
   ill-typed shapes at `-O0` and `-O2`, and classifies each compile as
   **ICE** (panic text in the output), **diagnosed**, or **silent** (exit 0,
   no diagnostic). `tests/regression/ill_typed_builtins_baseline.json`
   ratchets the first and third: a name that starts panicking, or that newly
   accepts an ill-typed call without a word, fails the gate; a name that stops
   panicking is reported so the baseline can be tightened in the fixing
   commit. `--self-test` exercises the ratchet itself (grow, shrink, hold)
   without a compiler, and it is wired into both CI mirrors.
   What it found, and what is left:
   * **Arity is fixed** for every name whose count the bundled headers pin
     down (303 of 624 — the `_mm*` wrappers from their parameter lists, the
     `__builtin_ia32_*` primitives from the call sites inside the wrapper
     bodies). `src/frontend/sema/intrinsic_arity.rs` is generated from
     `include/*.h` by `scripts/check_intrinsic_arity.py --write`, and
     `--check` (a CI gate) fails if the table drifts from the headers.
     Measured with the gate: **650 → 174 ICE (name, opt) pairs, no additions**,
     and 380 → 294 silent. GCC is the oracle for the wording and the verdict:
     `too few arguments to function '_mm_add_ps' (expected 2, have 0)`, and a
     header-declared name still reports exactly once (the prototype check wins
     when a declaration is in scope).
   * **Type classes are the remainder.** The headers give arity, not types, so
     `__builtin_ia32_paddb128(x, &x)` (right arity, wrong classes) still
     reaches the backend, where the operand indexing panics — 174 pairs today,
     concentrated in `__builtin_ia32_*` names the headers never call (93 of
     them have no call site) and in ops whose emitter reads an operand the
     lowering did not type-check. 294 pairs are *silently accepted* ill-typed
     calls, the AUD-DIAG-1 class (`_mm_loadu_si128(&int)` style pointer
     mismatches are legitimate — GCC warns, not errors — but the integer-class
     cases are not).
   * **The entry point for the next slice** is the same table: give it a type
     class per argument (GCC's own `-m` flag diagnostics are the oracle, and
     `gcc -c` on each shape says which argument is wrong: `incompatible type
     for argument 1 of '__builtin_ia32_addps'`), then extend
     `expected_arity` to `expected_signature` and enforce it in the same
     `builtins::is_builtin` arm of `src/frontend/sema/analysis.rs` that now
     enforces the count.
   The wider census (889 panic-class sites, 109 in the hot set) is unchanged
   by this work and still stands as the audit's item list for
   `x86/codegen/intrinsics.rs` (47), `slp_vectorizer.rs` (32),
   `x86/assembler/parser.rs` (29) — those are *backend* sites that only a
   front-end validation can keep unreachable from source.

3. **AUD-DEBUG-1 — `-g` still yields line tables only** (no
   `.debug_info`/type info, `-gsplit-dwarf` rejected). Minimal DWARF 5 for
   `-O0`: `DW_TAG_compile_unit/subprogram/formal_parameter/variable/base_type/
   pointer_type/structure_type` with `DW_OP_fbreg`; acceptance = gdb
   `bt`/`info locals`/`print` on a three-function sample, `readelf
   --debug-dump` clean, `llvm-dwarfdump --verify`. Below the performance work
   in priority, but it is the difference between "can be debugged" and not.
4. **AUD-EXPRID-1 — expression identity is a heap address.** `ExprId` wraps
   `&Expr as *const _ as usize`; `expr_ctype_cache` (`ir/lowering/lower.rs`,
   `expr_types.rs`) detects address reuse by comparing the `Expr` *variant*
   only, so a dropped `Expr::Binary` whose address is reused by another
   `Expr::Binary` of a different type can return a stale `CType` (silent
   wrong-type miscompile). No reproducer yet: step 1 is one — many
   `typeof(a+b)`/`_Generic` nodes of differing types in a function whose AST
   nodes are dropped between lowering steps, with a poisoning allocator behind
   a `CCC_` knob. Step 2 is the real fix: a parser-assigned `u32` counter in
   each `Expr` (call sites already use `expr.id()`), deleting the variant check
   and the two TODOs.
5. **AUD-ARM-GSF-1 — a parked ARM peephole miscompile.** Global store
   forwarding "produces wrong output" in complex float-array code and is
   skipped; four tests are `#[ignore]`d (`arm/codegen/peephole.rs`), and two
   x86 tests are ignored as "trampoline coalescing not triggering". The census
   now ratchets the *count* (baseline 1, because the ARM ones carry an
   annotation); either fix and re-enable, or delete the pass and its tests.
6. **AUD-MONO-1 — 192 functions over 300 lines, 23 over 1,000.** Split
   behaviour-preservingly, one function per change, proved by the repo's own
   identity gates (`check_codegen_refactor_identity.py`, encdiff/asmdiff,
   linker suite). Start with `emit_intrinsic_impl` (5,839) and
   `encode_mnemonic` (3,056), which are also where the ICE/SILENT-ZERO fixes
   land — splitting them first makes both easier.
7. **AUD-KNOB-1 — 498 `CCC_*`/`LCCC_*` literals, 292 with no documentation.**
   `python3 scripts/audit_census.py --list-undocumented-knobs` is the list. The
   audit's cost claim (≈550 `env::var` reads outside `LazyLock`/`OnceLock` in a
   compiler that lowers on a worker thread) is **unmeasured**: measure first
   (`ltrace -c -e getenv` on `sqlite3.c`) before building the registry.
8. **AUD-CI-1 remainder** — `ci.yml` is still one ~1,430-line `test` job
   (first failure hides every later gate, nothing runs in parallel), no
   `permissions:`/`concurrency:`/`timeout-minutes`, actions pinned to mutable
   tags, `runs-on: ubuntu-latest`. `scripts/check_ci_gate_parity.py` encodes
   the contracts that a split must preserve — run it after every step of the
   split, do not reorder it into the same commit.
9. **AUD-DOC-1 remainder** — `engineering/STATE.md` mixes GCC 14.2 numbers with
   GCC 16.2 oracle pins and references absent paths; 69 `/home/user`
   occurrences across 30 scripts need one shared env-with-auto-detection
   helper; four snapshot flavours still exist (this session used
   `scripts/lccc-snapshot.sh` as canonical, per the audit's own declaration);
   `scripts/README.md` should note that `-o -` and the temp-file behaviour are
   fixed as of AUD-DRV-1/AUD-TMP-1.
10. **AUD-BL-1** — the backlog file itself (upstream `PBT_TRANSPLANT.txt` /
    its backlog section) still mixes closed and open items and has a duplicate
    `IVOPTS-1`; each open entry should carry front matter naming its
    reproducer, with a gate that the path exists.
11. ~~**`sv_assign` miscompile (segfault at `-O2`)**~~ — **closed as
    unreproducible.** `git log -S sv_assign --all` turns up the follow-up
    document and nothing else: no `src/` occurrence, no symbol in the built
    rlib. The backlog entry names no reproducer, so there is nothing to fix
    until someone produces one; it is a documentation defect in the backlog
    (AUD-BL-1), not a known bug.

12. **AUD-VABI-2 — the 32-byte vector ABI (latched, gated, NOT fixed).** Found
    by running the AUD-VABI-1 four-way differential on a 32-byte vector
    typedef. The psABI classifies a vector as SSE plus one SSEUP per further
    eightbyte, so with AVX enabled a 32-byte vector travels in **one `%ymm`**;
    lccc's struct rule has no SSEUP notion and `CType::sysv_vector_carrier()`
    stops at 16 bytes, so the value falls back to the memory/sret convention.
    Measured with the four-way build in
    `tests/regression/vector_abi_boundary/` (`wide.c`, compiled by each
    compiler on each side, linked in all four combinations):

    | configuration | gcc/gcc | lccc/lccc | lccc caller + gcc callee | gcc caller + lccc callee |
    |---|---|---|---|---|
    | `-mavx2` | correct | correct | `add: 0 0 0 0 0 0 0 0` | **SIGSEGV** |
    | no `-mavx2` | correct | correct | correct | correct |

    The default code-generation baseline is x86-64-v3 (AVX2 on), so the broken
    column is the *default* one, and the caller-side failure is a silently
    wrong answer — AUD-VABI-1's class, one register width out.
    `tests/regression/check_vector_abi_wide.sh` **latches** the debt: the two
    working configurations are asserted to stay correct (the no-AVX2 four-way
    agreement is also the gate's liveness proof) and the two broken legs must
    show exactly the recorded verdict (`garbage` / `signal`) — a leg that
    changes, in either direction, fails the gate until its budget constants are
    updated in the commit that changed it.

    Fix shape (the AUD-VABI-1 fix, one width further out): a `%ymm` carrier for
    vector arguments and returns — `CType::sysv_vector_carrier()` (add 32, and
    64 for AVX-512), the signature classification in
    `src/ir/lowering/{lower,expr_calls,func_lowering,stmt_init}.rs`, the x86
    prologue/return path (`src/backend/x86/codegen/prologue.rs`) and
    `location_alloc` — then flip the gate's `DEBT_LCCC_CALLER`/
    `DEBT_GCC_CALLER` to `agree`.
13. **FMA contraction happens at `-O0`, unlike GCC.** With FMA permitted
    (the x86-64-v3 default), lccc contracts `a*b+c` even at `-O0` — measured:
    10 `vfmadd`/`vfmsub` sites in an `-O0 -march=x86-64-v3` build — while GCC
    contracts from `-O2` only. C leaves contraction implementation-defined, so
    this is legal, but it makes `-O0` numerics differ from GCC on FP-heavy code
    (this is what the `miscompile_gen_fp_stress_*` backlog family was actually
    measuring; all 24 agree once the ISA is pinned on both sides). There is no
    `-ffp-contract=` flag; adding it (and defaulting `-O0` to `off`) is the
    low-risk conformance win, and it makes the reproducer replay
    (`scripts/check_repro_replay.py`) simpler than its two pinned legs.

14. **AUD-CPU-4 — the 32-bit target emits 256-bit VEX code with no permission
    at all.** Measured on this tree (`-O2`, the x86-64 driver with `-m32`):

    ```
    -m32            -S f256.c (_mm256_add_ps)   -> vaddps %ymm1, %ymm0, %ymm0
    -m32 -mno-avx   -S f256.c                   -> vaddps %ymm1, %ymm0, %ymm0
    -m32 -mno-avx2  -S i256.c (_mm256_add_epi32)-> vpaddd %ymm1, %ymm0, %ymm0
    ```

    So `-mno-avx` does not stop VEX emission on i686 — the SIGILL class
    AUD-CPU-3 closed for x86-64, in the vector-op path rather than the
    intrinsic table. The same model refuses those intrinsics on i686
    (`x86_isa()` is `NONE` for anything but x86-64 by design), so the two
    halves of the permission story disagree: an intrinsic is refused while an
    equivalent operator is lowered to the same instruction. One model has to
    serve both, and it needs the 32-bit backend's own notion of a vector class.
    Latched by `tests/regression/check_intrinsic_feature_gate.sh`, which
    asserts the current emission in both directions, so the fix cannot land
    without updating those rows in the same commit.

---

## 3. Environment and harness rules this session had to relearn

* **The harness wipes `/home/user/artifacts`, `/opt/rust` and
  `lccc/target` and re-clones the repository.** Working-tree files survive,
  including uncommitted edits. Recovery: `git fetch origin <branch>:refs/remotes/origin/<branch>`
  then `git reset --mixed -q <ref>` — **never** `git checkout`/`--hard`, which
  would discard the surviving edits.
* **The Rust toolchain is not part of the image.** `pip download
  arena-rust-toolchain{,data1,data2,data3}==1.97.0`, concatenate the three
  parts (296,984,961 B), `zstandard`-decompress to a 932,915,200 B tar, extract
  to `/opt/rust` (`sudo mkdir -p /opt/rust && sudo chown user:user /opt/rust`).
  `arena_rust_toolchain.install()` cannot work: it hardcodes a
  `arena_rust_toolchain_data4` that is not on PyPI, and it needs a `zstd`
  binary the host does not have. Recipe: `/home/user/tools/restore_toolchain.sh`
  (+ `zstd-shim.py`). Build with `PATH=/opt/rust/prefix/bin:$PATH`.
* **Swap is mandatory**: `sudo fallocate -l 8G /swapfile`, `mkswap`, then
  `sudo /sbin/swapon /swapfile` (`swapon`/`mkswap` are not on `PATH`).
* **After a wipe the snapshot script anchors `BASE` at `HEAD`** (its
  `artifacts/.base_ref` is gone) and silently publishes a zero-byte patch. Pass
  `LCCC_BASE_REF=7e9444174f9b76181504329cd4ff8010b1a1483b`, and never accept a
  shrinking patch.
* **`scripts/lccc-snapshot.sh` is gate-aware**: it refuses to publish unless
  `ci_local.sh --fast` passed on *that exact tree* (`target/ci_local.pass`,
  keyed by tree hash). Budget a full run (~40 min) before every snapshot;
  `LCCC_SNAPSHOT_ALLOW_PARTIAL=1` exists for an interim autosave.
* `PyYAML` is needed by `scripts/check_ci_gate_parity.py`
  (`pip install --break-system-packages pyyaml`); without it the gate fails
  cleanly instead of silently passing.
* **Check a tool exists before writing a replacement for it.** `git ls-files`
  is the authority: `scripts/audit_census.py` and its baseline are committed
  and green (AUD-RATCHET-1 landed in `7b0ea71`); a rewritten copy would have
  silently replaced a better-specified engine (it also latches `#[ignore]`
  annotations, and its knob-documentation rule is "named in a tracked `*.md`").
* **Never edit a script while a bash process is executing it.** The local CI
  runner reads `scripts/ci_local.sh` incrementally by byte offset, so an edit
  in the middle of a run made the running shell die with `syntax error near
  unexpected token 'else'` at a line that is valid in the commit. Land the
  edit, then start the run — and treat a run that outlived an edit as
  evidence-free.
* **A new permission gate can invalidate existing tests whose flags were
  permissive.** `tests/regression/clmul256` carried `-mavx2 -maes -mpclmul`
  while emitting `vpclmulqdq`; the VAES/VPCLMULQDQ gate made that flag set
  illegal, and the corpus sweep is what caught it (`clmul256 (compile:fail)`).
  GCC 12.2 rejects the old set too, so the test's flags — not the new gate —
  were stale. After landing a gate: run `python3 tests/regression/run_regression.py
  --lccc ./target/fastbuild/lccc -j 2 --compile-only` and diff the failure list
  against the recorded baseline (8 failures, all `cannot find -lgcc`).
* **Silencing the oracle's own note can be right.** The wide-vector gate
  passes `-Wno-psabi` to gcc: in the no-AVX2 configuration the warning *is* the
  property under test (the memory convention), so it is expected output, not a
  finding.
* **Pin the ISA on both sides of a differential, or measure the wrong thing.**
  lccc's documented baseline is x86-64-v3 and GCC's default is x86-64; an
  unpinned comparison of the two reports 29 "failures" that are FMA
  contraction. State the baseline in the gate, in both directions.
* Gate files must avoid `producer | grep -q` and `producer | head -N` under
  `set -o pipefail` (the repo has `scripts/check_pipefail_sigpipe.py`, which
  caught exactly that in the new IVSR gate): capture first, or use
  `grep -c … >/dev/null` / `sed -n '1,Np'`.
* A commit message is checked against the tree at that revision
  (`scripts/check_pr_claim_fidelity.py`): naming a path that the same commit
  *moved* fails the gate, so describe moves without the old path.
* `tests/regression/*.c` is globbed **non-recursively** and each file must be a
  self-contained program; multi-file fixtures belong in a subdirectory (this
  is also what `run_regression.py`'s own failure text says).

## AUD-ICE-1 residual — measured composition and the design for the next slice

The arity half of AUD-ICE-1 pinned 303 names from `include/*.h` and took the
ratchet from 650 ICE / 380 silent to **174 ICE / 294 silent**
(`tests/regression/ill_typed_builtins_baseline.json`). The remaining 174 are
87 names x 2 optimisation levels, and every one of them is a name whose
argument count the headers do **not** pin, so the ill-typed call still reaches
codegen. Measured this session, not inferred:

* The panics come from two shapes, both reached with **zero** arguments: the
  five `assert!(args.len() >= N, "malformed intrinsic …")` guards in
  `src/backend/x86/codegen/intrinsics.rs` (lines 614, 8462, 9381, 9506, 10180)
  and direct `args[i]` indexing at other sites (883, 2768, 3325, …). Example:
  `__builtin_ia32_pxor128()` → `emit_sse_binary_128: malformed intrinsic pxor
  (0 args)`; `_mm_insert_epi16()` → `index out of bounds: the len is 0`.
* Why the count is unknown for these names, in two parts:
  `_mm_slli_epi32`, `_mm_shuffle_epi32`, `_mm_and_si128` and their 256-bit
  siblings are **function-like macros** in `include/*.h`, and the derivation
  only read *declarations* (its regex excludes `#` deliberately); and the
  `__builtin_ia32_*` primitives they expand to have **no call site at all** in
  the headers, because many wrappers are written with vector operators.
* Two measured levers, in increasing order of coverage:
  1. **Macro-aware derivation** — read `#define _mm_x(a, b)` parameter lists
     too (skipping variadic ones): 303 → **332** names, which pins 22 of the
     87 ICE names.
  2. **Variant propagation** — the front end's own name table (`builtins.rs`)
     maps both spellings of an intrinsic to one `BuiltinIntrinsic` variant
     (`_mm_and_si128` and `__builtin_ia32_pand128` are both `X86Pand128`).
     Propagating the header-derived arity along that mapping gives **216
     variants** an arity (4 dropped as conflicting) and covers **72 of the 87**
     ICE names. The 15 left over are variants with no header-derived public
     name at all (`X86Cmpps256`, `X86Rdtscp`, …): they need a different source
     (the emitter's own contract, or GCC's prototypes for the
     `__builtin_ia32_*` primitives).
* Design note for the slice: (2) wants a *variant*-keyed lookup, i.e. the
  generated file grows a `match variant { … }` beside the name-keyed table, and
  the sema hook in `analysis.rs` — which already holds the variant — asks for
  the variant's arity when the name is unknown. The backend asserts stay where
  they are: the audit's criterion is that they are unreachable from any
  ill-typed source, not that they are deleted, and they are the last line of
  defence behind the front end.

## CI-DIAG-1 — a red run must name its own cause (2026-10-13, evening)

### What was red, and what it actually was

An instructed "CI IS RED, fix it properly" opened a forensic pass over every
red run in `ms178/lccc`.  The finding is that **no red run was ours and none
was shared**: they are three other Arena branches, each failing for a
branch-local reason, on top of a repository-wide *observability* defect that
this branch now fixes.  The evidence, per branch (branch tip, failing job,
failing step, cause):

| Branch | Failing step | Cause | Verdict |
|---|---|---|---|
| `arena/a615b49b-lccc` #814 @`0a72c107` | Test Suite step 34 `Run tests` | 10 failing tests: 6 **pre-existing** (`passes::vectorize::map_fp_select_tests::{fold_max_from_sgt, fold_max_from_slt_swapped_arms, fold_min_from_sgt_swapped_arms, fold_min_from_slt, nested_clamp_parses, sle_select_does_not_fold}`) broken by that branch's vectorize/peephole edits, plus 4 tests that exist **only** on that branch (`load_op_fuse::tests::commuting_adjacent_fuse_{rehomes_a_single_family_rmw_product_update, keeps_full_width_self_move_reads_live}`, `minmax_reduction_legality_tests::marching_pointer_{preflight_tracks_only_the_exact_latch_step, scaling_changes_only_the_preflighted_step_definition}`) | Branch-local |
| `arena/074efb69-lccc` #816 @`727c3465` | Test Suite step 6 `Install the GCC 16 oracle` | Its own provisioning step fails before a single test runs (that workflow carries 17 `gcc-16` references where `main` has one comment) | Branch-local |
| `arena/24fe00ee-lccc` #817 @`ee247ff` | Test Suite step 96 `Run __seg_gs conversion diagnostics check` | Steps 52/53 `Run tests` **pass** there now; the surviving red is that branch's own segment-space gate.  Its paired `Audit diagnostics (temporary)` run 38476873196 reports success with `fail flag=0` because that workflow replays a **hand-maintained** gate list which does not contain the failing gate — a parity gap in the replay, not evidence of green | Branch-local |
| `arena/e588143d-lccc` #818 @`00394a81` | — | Runs were still in flight at the time of writing; nothing on that branch descends from ours | Unknown |

Measurements that make "branch-local" a fact rather than a reading:

* `main` @`3b66cd1d` and **this branch** @`299d89a2` are green in every
  workflow (`CI`, `Benchmarks`), and `Test Suite` on our head is 45/45 steps.
* The six shared tests #814 fails pass here: `cargo test --profile fastbuild
  --lib -j2 -- map_fp_select_tests` → `6 passed; 0 failed`.  They also pass on
  `main`, whose CI is green at that commit, so the break is inside that
  branch's diff, not in the base it started from.
* #814 does not descend from this branch and this branch does not descend from
  it (`git merge-base` = `3b66cd1d` for both), so no fix of ours can reach it.
* Our own earlier red, run 38464961178 @`ddb127bf`, was step 15 `Check -o -
  stdout semantics` — the merged `-o -` pair, already fixed by collapsing it to
  one guard plus one union gate; it is not part of this thread.

### Why a failing `cargo test` step was unreadable

Three facts, all re-verified this session:

1. `GET /repos/ms178/lccc/actions/jobs/{id}/logs` answers **HTTP 500** for a
   failed job (and 404 for the `commit/<sha>/checks/<id>/logs` HTML path); the
   public job page (≈286 KB of HTML) contains no verdict text.
2. `check-runs/{id}` returns `title`/`summary`/`text` = null even for a
   *successful* job, and the annotations a failed `cargo test` step produces
   are GitHub's own `Process completed with exit code 101` plus the artifact
   upload warning.
3. The one thing that *was* readable on #814 was an annotation emitted by a
   re-annotating wrapper that branch happened to write (`cargo test` piped
   through a shell helper; absent from `main`), which re-emitted the log tail.
   Without it, ten failing tests were invisible from the run page.

Consequence: hours of triage per red, and a real risk of "fixing" the wrong
side.  That is the defect this branch fixes.

### What landed

* **`scripts/ci_test_report.py`** — runs a command with stdout/stderr merged
  and streamed through byte-for-byte unchanged (exit status preserved,
  `128+n` on a signal), and on failure republishes the verdict as (a) one
  `::error` check annotation per failing test, placed on the panic's own
  `file:line`, (b) a summary annotation naming every failure **first** (GitHub
  shows only a bounded number of annotations per step, and the names are what
  must survive the cut), (c) a Markdown `$GITHUB_STEP_SUMMARY` with the
  complete list, the panic messages and the log tail, and (d) a local stderr
  digest so `ci_local.sh` prints the same verdict without a browser.  The
  parser understands libtest's `---- <name> stdout ----` sections, both
  `panicked at` spellings (including the `thread 'name' (15824) panicked at
  file:line:col:` form the real 2026-10-13 log used), the `test ... FAILED`
  marks and trailing `failures:` list, and rustc's `error[EXXXX]` / `error:`
  diagnostics with their `--> file:line:col` spans (including `-D warnings`
  promotions).  It never invents a cause: when nothing parsed, the tail is
  published verbatim.  `--parse-log FILE` replays a saved log.
* **Both mirrors wired**: the two `Run tests` steps in
  `.github/workflows/ci.yml` and the two cargo-test gates in
  `scripts/ci_local.sh` run through the wrapper, and the self-test runs first
  on each side (a reporting tool that quietly stops reporting is worse than
  none).  `check_ci_gate_parity.py` two-way parity stays green — no allowlist
  entry was added, on either side.
* **One misleading annotation removed**: the corpus-failure surfacing step
  printed `no regression-results.json was written` whenever *any* earlier step
  failed, which reads like a broken harness rather than "the corpus gate never
  ran"; it now says what is true and points at the failing step.

### Validation

* `python3 scripts/ci_test_report.py --self-test` — green; covers the escaping
  rules, a verbatim fixture of the real 2026-10-13 failure (thread-id panic
  header, assembly-dump message, both `failures:` blocks), both panic
  spellings, build diagnostics with two distinct spans and a `-D warnings`
  promotion, a diagnostic printed *inside* a test section (must not be read as
  a build error), CRLF + ANSI + invalid UTF-8 input, the annotation cap, the
  "nothing named itself" fallback, run-mode output passthrough and exit-code
  preservation (101 and 143-by-signal), and `--parse-log`.
* **Real libtest binary**, not a fixture: `rustc --test` on a three-test probe
  (one pass, one assertion with a debug dump containing `%`, one bare panic),
  run through the wrapper with `GITHUB_ACTIONS=true` → exit `101`, two
  annotations on the probe's real `file:line`, complete job summary.
* **The real suite through the same path**:
  `bash scripts/ci_local.sh --only cargo-test` → `PASS cargo-test` and `PASS
  cargo-test-debug-assertions` (329 s), 0 failed.
* Gates: `check_ci_gate_parity.py`, `test_ci_gate_parity.py` (66 tests),
  `check_ci_workflow_shell.py`, `check_script_imports.py`,
  `check_no_conflict_markers.py`, `check_doc_links.py` — all green.

### ToDo for the red branches (this branch cannot push to them)

1. `arena/a615b49b-lccc`: fix the ten failing tests listed above — six are
   pre-existing contracts that its own changes broke.  Nothing in `main` or in
   this branch needs a change for them.
2. `arena/074efb69-lccc`: repair or drop the failing GCC-16 provision step; a
   gate whose provisioning fails is not evidence about the compiler.
3. `arena/24fe00ee-lccc`: fix `__seg_gs conversion diagnostics`, and make the
   `Audit diagnostics (temporary)` workflow derive its gate list from
   `ci.yml` — a replay that reports `fail flag=0` while the real suite is red
   assumes the very parity it is supposed to prove.
4. Any of them: rebase on this branch once it merges, and the next red names
   its own failing tests on the run page.
