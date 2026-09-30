# LCCC Engineering Backlog

Live triage queues for measurable runtime/quality work. This file holds
**open items and their evidence**; session narratives, landed-work
history and measurement method live at the paths below — add history
there, not here.

- Sessions / measuremethod: [`engineering/journal/`](engineering/journal/README.md)
  (worst-10/worst-12 triage screens: `2026-09-W2.md`; daily entries tagged
  per-workstream: per-day edit).
- Method doctrine: [`engineering/agent/RULES.md`](engineering/agent/RULES.md).
- Per-area canonical knobs/kill-switches:
  [`engineering/subsystems/`](engineering/subsystems/README.md).
- The measured oracle baselines: [`engineering/evidence/benchmarks/`](engineering/evidence/README.md#benchmarks-and-comparisons).
- Deep research: [`engineering/DECISIONS.md`](engineering/DECISIONS.md) (measured
  negative-space, do-not-retry grounds), [`ideas/`](ideas/README.md).

Last broad triage rebuild: **2026-09-16**, fictional base `8ca2fd4`.
The historical numbers below are from that earlier screen, not the current
baseline. On **2026-09-25**, `main` at `7ddb770f` was remeasured in 39
randomized paired, output-checked VM workloads against the candidate and GCC;
the only changed executable `.text` was LZ4. The sound byte-copy loop-idiom
fix retains ~10.8× speed over disabling that idiom on a separately scaled
LZ4 screen, while preserving forward-overlap semantics. See
[`engineering/evidence/2026-09-25-redteam/loop-idiom-redteam/README.md`](engineering/evidence/2026-09-25-redteam/loop-idiom-redteam/README.md).
On **2026-09-26**, follow-up work was based on upstream `e5bc1911`. A
41-workload randomized three-arm Xeon-VM screen found changed executable
`.text` only in `spectral_norm`; the paired 15-round focused result is
candidate/current-main **0.0361** (240 ms / 6.60 s), with exact benchmark
output agreement. The fix replaces the AVX2 strict-reciprocal lane pack's
legacy SSE instructions with VEX forms. The newly added match-rich LZ4 and
scaled find-bit arms reveal gaps hidden by the original short/no-match inputs.
The evidence and remaining P0 limitations are in
[`engineering/evidence/2026-09-26-followups/README.md`](engineering/evidence/2026-09-26-followups/README.md).
The i7-14700KF target has **not** been measured. Before accepting a new
**target-hardware** runtime improvement, use same-window, same-baseline,
output-checked paired A/B with ≥ 200 ms/arm and agreeing median/min, not
historical ratios.

An item with no reproducer does not belong here.

---

### ALIGN-1 · **NEW 2026-09-30** — `aligned()` on a function *definition* is ignored

Found by making rustc enumerate dead state (`unused_assignments` was un-allowed
crate-wide): `parser/nested_functions.rs` parsed `aligned(...)` off a nested
function's declarator, combined `decl_aligned` with `post_aligned` into an
`alignment`, and nothing could consume it.

The mechanism, verified: function alignment reaches codegen through
`IrModule::function_alignments`, keyed by the **plain** function name
(`ir/lowering/global_decl.rs` inserts from a prototype's `decl.alignment`;
`backend/generation.rs:3706` emits `.p2align` for a *definition* out of that
map), while a nested function's IR name is **mangled** `parent.inner`
(`ir/lowering/nested_functions.rs:879` and `:1258`). Any value computed at the
definition site therefore cannot match the lookup.

Consequence today: `__attribute__((aligned(N)))` on a function **definition**
(top-level or nested) does not reach the emitted `.p2align`; only a *prototype*
carries it. GCC honours the definition form.

Deliberately NOT shipped this session: the dead computation was removed and the
discard documented in place (the drop is declared intentional at the
`FunctionDef` construction), because wiring it needs the mangled key and a
codegen assertion, not a one-line insert. The dangerous fix — deleting the
computation and saying nothing — would have cemented the dropped attribute as
intended behaviour.

**MEASURED 2026-09-30** (one file, `-O2 -S`, GCC 14 as oracle; the directive
preceding a label is the one that positions it):

| function in `align_test.c` | attribute on | GCC | LCCC |
|---|---|---|---|
| `via_def` | definition only | `.align 64` | `.p2align 4` (**ignored**) |
| `via_proto` | prototype | `.align 64` | `.p2align 6` |
| `plain` | none | `.p2align 4` | `.p2align 4` |
| `proto_then_def` | prototype + definition | `.align 64` | `.p2align 6` |

So the gap is exactly the definition channel, and the prototype channel works.
Reproduce with:
`printf '%s\n' '__attribute__((aligned(64))) void via_def(void) { }' \
  '__attribute__((aligned(64))) void via_proto(void);' 'void via_proto(void) { }' \
  'void plain(void) { }' > align_test.c && target/fastbuild/lccc -O2 -S align_test.c -o -`
and compare against `gcc -O2 -S`.

**Second observation, cosmetic but worth knowing:** for a function whose
alignment *does* come through the map, LCCC emits two directives where GCC emits
one — `.p2align 6` (the attribute) followed by `.p2align 4` (a default emitted by
another layer). Harmless, because the second pads nothing after the first, but it
hides which directive carries the attribute from anyone reading the asm. Worth
folding into the same fix.

**First step for the fix:** the mangled key (`parent.inner`) plus a codegen
assertion; the characterisation above is the acceptance test — the
`via_def` row must move to `.p2align 6` while the other three stay put.

### ALIGN-1 · **CLOSED 2026-09-30** — the definition channel of `aligned()` now reaches codegen

Was: `__attribute__((aligned(N)))` on a function **definition** was parsed and
dropped, so only a *prototype* reached `IrModule::function_alignments` and thus
the emitted `.p2align`. Measured against GCC 14 (which honours both channels):

| function | attribute on | before | after | GCC 14 |
|---|---|---|---|---|
| `via_def` | definition only | `.p2align 4` | `.p2align 6` | `.align 64` |
| `via_proto` | prototype | `.p2align 6` | `.p2align 6` | `.align 64` |
| `plain` | none | `.p2align 4` | `.p2align 4` | `.p2align 4` |
| `proto_then_def` | prototype + definition | `.p2align 6` | `.p2align 6` | `.align 64` |

Fix: `FunctionDef` carries its own `alignment`; the lowering registers it under
the **emitted** name (the asm label when one exists). Gate:
`tests/regression/check_function_alignment_definition.sh`, verified to fail on a
stashed-fix build in exactly the one cell that changed, with `plain` as the
control against a blanket alignment change.

**Non-goal, measured:** nested function definitions. GCC applies no observable
alignment to one (`-O0`/`-O2`, with and without `noinline`), so there is no oracle
to match; LCCC ignores it too, and the wiring that made it honour the attribute
was tested, found not to fire, and removed rather than shipped.

### ALIGN-2 · **NEW 2026-09-30** — a redundant trailing `.p2align` hides which directive carries the attribute

For a function whose alignment comes through `function_alignments`, the asm is

```text
    .p2align 6        <- the attribute
    .globl f
    .p2align 4        <- a default, emitted by another layer
f:
```

Harmless — the second pads nothing after the first — but it makes the attribute
invisible to anyone reading the asm, and it made a naive gate fail. GCC emits a
single directive. **First step:** find the emitter of the second directive (it is
not the `function_alignments` site) and suppress it when the first already
applied.

### VOLATILE-BF-1 · **CLOSED 2026-09-30** — a volatile bitfield emitted non-volatile accesses

Eight hardcoded `volatile: false` sites in three lowering functions
(`store_bitfield` full-width branch, all four of `store_bitfield_split`, all three
of `extract_bitfield_from_addr`) meant an object-level volatile bitfield was
accessed non-volatilely. At -O2 that is a wrong answer, not a lost optimisation:

| probe | before | after | GCC 14 |
|---|---|---|---|
| N volatile full-width stores in a loop | 1 store | N stores | N stores |
| 4 unrolled volatile bitfield reads | 1 load | 4 loads | 4 loads |

Volatility is now resolved once at member-access entry and threaded to all three
functions; a split-load-only asymmetry is impossible by construction. Gate:
`tests/regression/check_volatile_bitfield_split.sh` (3 FAIL before, 0 after,
non-volatile twins as controls). The ratchet that would have caught the rename
that hid this is now in `scripts/check_volatile_destructuring.py` (parameter
rule, self-tested against the old signature verbatim).

### DEAD-BIND-1 · **CLOSED 2026-09-30** — 86 underscore bindings classified, 44 removed, 42 justified

`git grep -nE 'let _[a-z]' -- src/` returned 86. Deleted 44: pure dead statements
(a whole per-function map, two never-called closures, several aliases), plus the
call-preserving rewrites where the *expression* had effects
(`narrow_function(&mut func)`, 12× `add_shstrtab_name`, `parse_expr(lx)?`,
`fields.next()?`). One was a control-flow guard in disguise
(`let _out_name = match .. { None => continue }`), rewritten as a guard.
Kept 42, all RAII windows whose Drop restores state
(`EnvGuard`/`ScopedFlag`/`TriStateFlagWindow`/`EnvWindow`/`RestorePointerSize`/
`Target::set`/`LateMinMaxOnlyScope`) plus one uninitialised declaration.
Removing the dead `_iv_width_const` made its parameter unused, which the enforced
lint caught and which was then removed too — the cascade is the point.

### ALIGN-3 · **CLOSED 2026-09-30** — the attribute is honoured with parameters, and it replaces the default

Two follow-on defects in the same feature, both found by testing rather than by
the gate that was supposed to cover it (that gate's fixture was parameterless,
which is why it passed while both were live):

1. **A parameter list swallowed the attribute.** The alignment pending when a
   parameter list begins belongs to the ENCLOSING declaration, and the
   per-parameter attribute capture merged into that slot and took it. Measured:
   `aligned(64) void f(int x) { }` → `.p2align 4` where GCC emits `.align 64`;
   same for the prototype channel and `_Alignas`. Only the parameterless spelling
   ever worked. Fix: the list is parsed with the value held aside and restored at
   a single exit point.
2. **An attribute REPLACES `-falign-functions`, it does not merge with it.** GCC,
   `-O2`, `-falign-functions=32`: `aligned(2)` still `.align 2`, while an
   unannotated neighbour gets `.p2align 5`; `aligned(1)` emits **no directive**.
   LCCC emitted the attribute directive and let the default follow, so
   `.p2align` (which only advances) made the pair mean max(N, 16) — `aligned(2)`
   was 16-byte aligned and `aligned(1)` was 16-byte aligned. Fix: one value in
   one place, `Option<Option<u32>>` (absent / fixed / natural), consumed by every
   placement site; this also yields the single directive per entry that GCC
   emits.

Gate: `check_function_alignment_definition.sh` 4 → 14 assertions (parameterized
channels, replace-not-merge rows, one-directive-per-entry counts, two
unannotated controls), mutation-proven in both directions. Object-verified with
GNU as: after a one-byte pad, `aligned(2)` at offset 2, `aligned(1)` at an odd
offset, unannotated at a 16-byte boundary — as GCC's own `.s` produces.

### CI-FLAKE-1 · **CLOSED 2026-09-30 (burn-down continues)** — a gate reported SIGPIPE as a failure

`set -o pipefail` + a consumer that stops reading early (`head`, `grep -q`,
`grep -m1`, `grep -l`) makes the producer's SIGPIPE (141) the pipeline's status,
so a gate can print *"pattern not found"* about a comparison that succeeded.
Measured: 27/20000 false failures for `echo | grep -Eq`, 153/20000 for
`printf | grep -Eq`, **0/20000** for a here-string or `[[ =~ ]]`; reproduced end
to end as a 1-in-3 flake on the volatile gate against a byte-identical binary
(400 compiles, 1 distinct output), with `PIPESTATUS` showing `echo=141 grep=0`.

Fixed in every script CI runs: 47 pipelines whose consumer was `head` →
`sed -n '1,Np'`, 29 whose consumer was `grep -q` → `grep -c … >/dev/null` or a
here-string — 76 sites in 23 gates, one linker helper and the CI driver; each
keeps the producer's real status and reads to end of input.
`scripts/check_pipefail_sigpipe.py` (13-case adversarial self-test) enforces
it, wired into `ci.yml` and `ci_local.sh`. Verified scope:
89 shell scripts scanned, 0 gates that CI invokes left unscanned; previously
flaky gate now 0/30 runs.

**Remaining burn-down:** 82 sites across 30 developer-facing helper shell
scripts (`scripts/repro_claims.sh` 12, `tests/linker/setup_oracles.sh` 12,
`tools/linker/setup_oracles.sh` 7, `tests/regression/check_global_addr_cse.sh` 6,
…). `python3 scripts/check_pipefail_sigpipe.py --census` prints the list. These
are not CI reds, which is why they are a list rather than a gate.

## P0 — largest measured gaps

### IVOPTS-1 · **NEW 2026-09-30** — index-form addressing is never strength-reduced

The largest single measured codegen defect, and the one that explains most of
the honest corpus deficit (see
[`ORACLE-METRIC-1`](engineering/evidence/ORACLE-METRIC-1/README.md): +15.8 % vs GCC over
the 47 non-recursion benchmarks, median per-file ratio 1.17).

`nbody`'s inner loop — 5 bodies × 5 000 000 iterations, the hottest loop in the
corpus — is **110 instructions against GCC's 14 (7.9×)**, with **zero** stack
references, so it is not register pressure. Per iteration we emit:

| waste | count/iter | GCC's equivalent |
|---|---:|---|
| `imulq $56, %r9, %r15` — index×stride by multiply | 2 | `addq $56, %rax`, once |
| `leaq bodies(%rip), %rcx` — static base re-materialised | 2 | hoisted to `%r12` outside |
| `cmpl $5000000, -72(%rbp)` — outer bound from a stack slot | 3 | register |
| `movsd`/`movupd`/`movq` data movement | 56 of 110 | displacement addressing |

GCC walks the array with one pointer bump and reaches every field through a
displacement (`vsubsd 8(%rax), %xmm7, %xmm2`). We stay in index form, recompute
`i*56` twice, and move the results around. The signature is a missing
**induction-variable strength reduction / IVopts** stage plus weak loop-invariant
address hoisting, and it is the same signature in `spectral_norm` (+114 %),
`moving_stats` (+66 %), `struct_copy` (+62 %) and `matmul` — the struct-array
and FP kernels that dominate the real deficit table.

Corpus-wide mnemonic census (51 programs, `-O2 -march=x86-64-v3`):

| pattern | LCCC | GCC | ratio |
|---|---:|---:|---:|
| `movsd` scalar FP move | 171 | 54 | **3.2×** |
| `leaq sym(%rip)` static base | 145 | 91 | **1.6×** |
| `imul $const,` index scaling | 87 | 55 | **1.6×** |
| stack refs | 655 | 773 | 0.85× (we are better) |

Work order, cheapest first, each independently measurable:
1. Hoist loop-invariant `leaq sym(%rip)` bases out of loops (145 → ≤91 target).
2. Strength-reduce `idx*stride` on a unit-step induction variable to a stride
   bump; prefer displacement addressing over materialised addresses.
3. Coalesce the scalar FP register-to-register moves (171 → ~54 target).
Do **not** start from the register allocator: these functions do not spill.

Reproduction: `python3 scripts/oracle_asm.py tests/benchmark/programs/nbody.c
--function main --flags "-O2 -march=x86-64-v3"`, then compare the inner loop
against `gcc -S -O2 -march=x86-64-v3`.

### METRIC-1 · **CLOSED 2026-09-30** — the oracle metric was ranking an inlining artifact

`codegen_oracle.py --rank` reported a 1899-instruction deficit, worst first
`zlib_ng_adler32::main` "204 behind icc=73". ICC's `main` is 73 instructions
because ICC left `zlib_ng_adler32_c` out of line as two copies the
single-function view never measured; whole translation unit, ICC is 43 % larger
than us over the six files that table put on top. 16 of the 77 "behind" rows
compared functions with different call counts and carried **781 of 1899 = 41 %**
of the headline, including both top rows.

Closed by adding a `calls` column, a per-row comparability marker, and a warning
naming the rows that cannot be ranked; `jmp`/`b`/`j` are deliberately not
counted as calls because a tail jump to an out-of-line copy is exactly the shape
that fakes a smaller function. Guarded by
`scripts/test_codegen_oracle.py` (11 cases, mutation-verified both ways) and
registered in both CI drivers. Full method and numbers:
[`ORACLE-METRIC-1`](engineering/evidence/ORACLE-METRIC-1/README.md).

Two corollaries that outlive the fix: a corpus **total** is the wrong statistic
here (four recursion benchmarks where GCC explodes carry the whole aggregate —
all-51 says −12.5 %, the 47 say +15.8 %), so quote the **median per-file ratio
and the larger/smaller counts**; and static instruction count remains a
screening metric only, never PMU evidence.

### PF-SN-1 · AVX2 strict-reciprocal pack, target confirmation pending
Current main `e5bc1911` emits legacy `movd`/`pinsrd` immediately before
`vcvtdq2pd`/`vdivpd`. The follow-up replaces these with VEX forms **only
when AVX2 is available**; a focused 15-round, output-checked Xeon-VM screen
found candidate/main 0.0361 (240 ms vs 6.60 s), and a 41-workload scan found
changed `.text` only in `spectral_norm`. This is a reproducible VM result,
**not** an i7-14700KF timing. Target P-core validation is pending; use the
runner and evidence linked above. Negative-trip, repeated-vector, scalar-tail,
changing-sign divisor, and XMM-disabled tests protect semantics.

### PF-LZ4-1 · Byte-copy semantics fixed; byte-compare still open
The default-on v2 copy matcher covers both relevant LZ4 loops. The old
unconditional `memmove` rewrite was **incorrect** for forward-overlap smear,
and naively disabling the pass caused a 10.8× scaled-workload slowdown on the
Xeon VM. A guarded memmove fast path now preserves the scalar overlap loop.
The original input executed **zero** match extensions in an instrumented
12,288-pass run; its near-parity time is *not* evidence for byte-compare parity.
A separately registered match-rich arm exercises 49,299 long matches in
24 passes. On current main, the 9-round scaled Xeon-VM LCCC/GCC 14 ratio is
~1.29; candidate/main `.text` is identical, so the byte-compare gap remains
**open**. Widening a compare without proving **both** operand ranges valid
can overread near an object boundary. No speculative widening or incorrect
copy rewrite shipped. Require sanitizer/page-edge and overlap tests, a range
proof, all four oracles, and an output-checked target i7 paired A/B.

### PF-MB-1 · Mandelbrot hot scalar FP loop (open)
Current `e5bc1911` 9-round Xeon-VM ratio to GCC 14 is ~1.30; candidate and
main `.text` match. GCC 16, Clang 23.1, ICC, and ICX oracles **also use scalar
FP instructions** in the compared main loop; static AVX-instruction counts
do not prove vectorization. Further cross-pixel widening needs legal SSA/exit
proof, and scalar loop/scheduling improvements require paired measurements.
Reproducer: `tests/benchmark/programs/mandelbrot.c`. Done = output-verifying
paired A/B with a proven general change on the i7-14700KF.

### PF-FB-1 · `linux_find_bit` loop structure (open)
Current `e5bc1911` scaled 9-round Xeon-VM ratio to GCC 14 is ~1.40;
candidate/main `.text` matches. `bsfq` is present; the open question is
branch/index structure, not the loop-copy idiom. A 21-round output-checked,
one-instruction assembly splice removing an apparently redundant post-`andn`
`test` gave median no-test/original 1.013 on this VM; **not** a win, so no
peephole was shipped. Done = classified, semantics-proven general CFG change
with output-checking and target-CPU paired evidence, or a justified bound.

### RA-GLA-04 · GLA Phase 2 — full-identity register remat (sha256 hot)
Phase 1 shipped (`CCC_RA_GLOBAL_LOCATION=1`); source-less remat is default-ON.
The checked Phase-2 *spill-gap* experiment was **not** shipped: paired
`sqlite_varint` and `expat_xml_scan` regressed 1.60% and 4.51% respectively
on an earlier VM despite a few better static counts. On current `e5bc1911`,
`sha256_transform` is ~1.18× GCC 14 on the 9-round Xeon-VM screen;
candidate/main `.text` is identical and its runtime ratio's interval spans
1.0. **No allocator speedup or regression is established.** Require
post-allocation slot-traffic feedback and a small, verified general change
before reconsidering full register-source-spanning remat.
**2026-09-28: the slot-traffic feedback prerequisite now exists** —
`scripts/stack_census.py --per-slot N` reports per-slot load/store/addr
traffic, and the census shows `sha256_transform` has **0 spill** and 12
`csave` references, so the Phase-2 remat experiment cannot pay for itself on
this function in its current shape. Re-open only for a function the census
shows as genuinely spill-dominated.
Oracle targets: `sha256_transform ≤ 1.5×`, epilogue ref-count −50 %.
Design refs: [`engineering/DECISIONS.md`](engineering/DECISIONS.md)
RA-GLA-01/02/03. Aligns with **R3** (RA span supply + phi/ORI lowering —
skip the 1.93× sha256 without tripling the web count; compare
GHASH/mulpack for the multi-source shape rule).

## Tier 1 — measured, largest first

### MINMAX-1 · **LANDED 2026-09-29** — integer min/max reductions on x86-64
`min`/`max` over `int[]` stayed scalar on x86-64 although the packed
machinery was finished: the reduction only becomes a `Select` after
`if_convert`, which runs AFTER the main vectorizer, and the late rerun that
catches it was gated on `matches!(target, Aarch64)` while its own comment
promised the x86-64 rerun. Fixed (shared early/late dispatch table, opened
to x86-64), plus `ReductionKind::Min` (`vpminsd`, new
`VecHorizontalMinI32x8`) and admission of the ordinary IV-indexed-GEP shape
at `iv_init == 0`.
Measured `-O3 -march=x86-64-v3`, steady-state density: `min_i32`/`max_i32`
**1.5 -> 0.1250 insn/byte** (12x; level with gcc 16.2 and icx, behind clang
23.1's 4x-unrolled 0.0547 only). Pinned oracles: clang 23.1 0.0547, gcc 16.2
0.1250, icx 0.1250, icc 0.5625.
Second defect found and fixed on the way (MINMAX-1b): a dynamic loop bound
was divided by the vector width with a `UDiv` inserted into the loop HEADER,
so the min/max steady state carried a real 64-bit `divq` — 6 of its 10
instructions. Now emitted once in the preheader as `LShr` (power-of-two
width): 10 -> 4 insns per trip. Both the AVX2 and the SSE2 transform, corpus
A/B shows no regression. See
[`engineering/evidence/MINMAX-1/README.md`](engineering/evidence/MINMAX-1/README.md).

* **MINMAX-5 · extract the duplicated dynamic-limit hoist (LOW-3).** The
  ~110-line hoist is cloned verbatim between `transform_reduction_avx2` and
  `transform_reduction_sse2`. Mechanical, and the right call eventually, but
  it must be its OWN commit with its own re-measurement: it touches the single
  largest behavioural change in MINMAX-1, and spending a correctness session's
  validation budget on a maintainability wart with no defect behind it is the
  wrong order. Deferred deliberately, with the reasoning recorded, rather than
  skipped silently.

**Gate/observability follow-ups from the PR #681 audit** (detail and
measurements in
[`engineering/evidence/PR681-AUDIT-RESPONSE/README.md`](engineering/evidence/PR681-AUDIT-RESPONSE/README.md)):

* **OBS-1 · `src/lib.rs` carries `#![allow(unused_variables)]` crate-wide.**
  This, not any rustc limitation, is why a deleted `!*volatile` guard in
  `licm.rs` reached a green Clippy job: rustc *does* lint refutable-pattern
  bindings (verified: `rustc -D warnings` errors on the exact shape), and this
  crate has the lint switched off. Until it is addressed, a dropped
  observable-access guard in any pass is invisible to the build.
  `scripts/check_volatile_destructuring.py` covers the one field where silence
  is a miscompile. The right follow-up is a **counted ratchet** over
  `unused_variables` sites — measure today, never raise it — in the style of
  `check_env_test_hygiene.sh`, migrating the safety-relevant sites first.
* **OBS-2 · widen the ratchet to the other observable-access fields.**
  `Instruction` carries a second flag, `semantic_volatile`, which has the same
  silent-drop exposure and currently **no instrument at all**. `AtomicLoad` /
  `AtomicRmw` / `AtomicStore` are excluded on the *argument* that `_Atomic`
  accesses are already unremovable; that is an argument, not a measurement.
* **OBS-3 · `check_volatile_spin_loop.sh` asserts on the INNERMOST loop.**
  A volatile access hoisted from an inner loop into an outer one would pass.
  Not reachable today (no pass sinks outward), but the helper should support
  "inside any enclosing loop" and the gate should say which it means.
* **OBS-4 · `loop_preheader` is 528 lines of CFG surgery, default-on at -O2,
  and fires ~0 times at default settings.** Measured: byte-identical output
  with and without it on every guard-at-top shape, including the SQLite
  `if (p == 0) return 0;` case its own docstring cites; it fires under
  `CCC_LOOP_ROTATE=1` (18x, per the W3 journal) and on a plain `-O2` `do`-while
  shape. Either find the shape family it is actually good for and measure the
  insertion rate over the golden workloads, or gate it off by default. Today
  it is paid for on every `-O2` compile and its value is invisible.
  `tests/regression/check_loop_preheader.sh` now pins both directions.
* **OBS-5 · an access COUNT cannot see a HOIST.**
  `check_volatile_pointer_subscript.sh` passed green on a compiler that hoists
  a volatile MMIO load out of its spin loop, because hoisting preserves the
  count. Measured, not assumed. Any future gate asserting on emitted code
  should ask about POSITION; `tests/regression/lib_loop_bounds.sh` is the
  shared primitive.

* **OBS-6 · `reloc_pc32_out_of_range_diagnosed_on_script_path` cannot pass on
  a host whose non-reference linkers cannot parse the fixture's script.**
  Measured on this box: 300 pass / 1 fail, with
  `only 1 of 2 oracles could express an opinion, need 2` -- bfd refuses, and
  mold 2.37 answers `unknown linker script token` for the fixture's
  `ENTRY(probe)`, so it is `inapplicable` and the applicability floor of 2 is
  never met. This is the fail-closed rule working **as designed** (see the
  "two inapplicable out of three is NOT a cross-check" known-answer case), and
  it is not a regression: on a host with bfd + lld + wild it passes. The
  defect is in the *fixture*, not the rule: a fixture whose linker script
  needs a token some oracles lack makes the test host-dependent in a way that
  reads as "lccc is unconformant". Fix by giving the fixture a script every
  configured oracle can parse, or by reporting an incapable oracle as an
  explicit `SKIP` for the case with the reason attached, rather than folding
  it into a FAIL that reads like a conformance verdict. Do NOT lower the
  floor.

**Named follow-ups, in value order** (all measured, all refused today so
they stay CORRECT rather than fast):

* **MINMAX-4 · the rest of the late rerun (measured, deliberately off).**
  The unrestricted rerun also fires the Adler-32 epic and the guarded-sum
  transforms on `zlib_ng_adler32`. Measured on the benchmark program itself
  (`-O3 -march=x86-64-v3`, `valgrind --tool=callgrind`, whole program):
  **396,349,832 -> 400,499,785 retired instructions (+1.05 %)**, static
  instructions 335 -> 370 (+10.4 %), stack references 32 -> 44 (+37.5 %) —
  even though the steady-state loop it produces is denser (14 -> 12 insns per
  32 bytes). The prologue, the extra accumulator traffic and the spills cost
  more than the packed body saves *on this workload*. The rerun is therefore
  scoped to min/max (`LateMinMaxOnlyScope`), which keeps the 12x min/max win
  and leaves every other kernel byte-identical (adler32 is now 332 static
  instructions and 0.3750 insn/byte — better than the 335 / 0.4375 baseline,
  from the preheader-division fix alone). Re-open with a PROFITABILITY guard
  (prologue + epilogue cost vs trip count), not by deleting the transform.

* **MINMAX-2 · multi-accumulator min/max.** `for (...) { if (a[i]<mn) ...;
  if (a[i]>mx) ...; }`, and the same loop with a sum. Two of the corpus's
  worst kernels are this shape (`moving_stats`: lccc 149 insns/trip vs icc
  0.625 insn/byte; `fir_filter`, `conv_u8_3x3` are nearby). The pattern
  models ONE accumulator, so it refuses; the follow-up is to populate
  `SecondaryAccumulator` with a `kind` and wire the min/max epilogue per
  accumulator. The refusal exists because NOT doing this produced
  `sum == 0` for every n below the vector width.
* **MINMAX-3 · 16-bit and unsigned lanes.** `vpminsw`/`vpmaxsw` are SSE2
  baseline (`vpmin*`/`vpmax*` for 8-bit need SSE4.1, `vpminud`/`vpmaxud`
  too). `moving_stats` is `short` data: lccc 3.0 insn/byte vs gcc 0.125.
* **LOOP-PREHEADER-2 · fix `loop_rotate`'s condition lowering, then enable
  rotation (measured blocker).** `loop_rotate` makes the loop body the header,
  which is what turns LICM's derived-pointer load hoisting from dead to live —
  with rotation plus the dedicated preheaders from LOOP-PREHEADER-1 the
  `fir_filter` FIR hot loop goes **56 -> 38 instructions (-32 %)**. Rotation is
  OFF by default because it is **+209 static instructions (+2.6 %)** across the
  51 benchmark programs at `-O3 -march=x86-64-v3`, and the entire cost is one
  lowering defect: the rotated latch materialises the loop condition as an `i1`
  value instead of keeping a comparison, so the backend emits `setl %bl;
  movzbl %bl,%ebx; testb %bl,%bl; jne` where a single `jl` belongs. Three
  instructions per iteration, in every rotated loop. Fix the condition
  lowering, re-measure, then re-evaluate enabling rotation at `-O2+` — which
  would simultaneously make LOOP-PREHEADER-1 pay corpus-wide. Evidence:
  [`engineering/evidence/LOOP-PREHEADER-1/`](engineering/evidence/LOOP-PREHEADER-1/README.md).
  **Blocking defect, already known:** `loop_rotate` leaves invalid SSA on 2 of
  the 51 benchmark programs (`fir_filter.c`, `moving_stats.c`): under
  `CCC_VALIDATE_SSA=1` both abort with `SSA PHI-ARITY VIOLATION after phase
  'after iter=0 loop_rotate'` — phi incoming-label sets that do not match the
  rewritten CFG. Verified NOT caused by LOOP-PREHEADER-1 (reproduces with
  `CCC_DISABLE_PASSES=loop_preheader`, and fires before that pass runs). Fix the phi
  rewriting before touching the condition lowering.

* **LOOP-PREHEADER-3 · ~~the pass has no dedicated gate~~ CLOSED.**
  Landed: `tests/regression/check_loop_preheader.sh` +
  `tests/regression/loop_preheader_shapes.c`, wired into `ci_local.sh` and
  `ci.yml`, and the shapes below are asserted on **emitted assembly**
  (a preheader that is structurally valid but hoists nothing is invisible to
  every runtime comparison and every Rust-side unit test).
    1. *Effect.* `guarded_sum` (the SQLite NULL-guard shape) loads `p->nUsed`
       once, outside the loop.
    2. *Soundness.* That load sits **after** the `p == 0` early return; hoisted
       into the guard block instead it would dereference NULL.
    3. *Negative control.* Under `CCC_DISABLE_PASSES=loop_preheader` the same
       loop reloads `p->nUsed` every iteration. Without this delta a pass that
       never fired would also show one load outside the loop and pass
       vacuously -- verified by disabling `enabled()`: three contracts fail.
    4. *Refusal.* Four shapes stay byte-identical between the two arms:
       `invariant_ptr` (nothing to unlock), `switch_entered` and
       `computed_goto` (the miscompile shapes -- counting only Branch edges
       would claim one of two entering blocks as *the* preheader, leaving the
       hoisted bound's def not dominating its use on the other path), and
       `already_dedicated`.
    5. *Idempotence.* With `CCC_DEBUG_LOOP_PREHEADER=1` the fixpoint inserts
       once and then reports "already has a dedicated preheader".
  Note on the spec's original first shape,
  `int f(const int *c,int n){int t=0;for(i<n;i++)t+=c[0];}`: it is **not** a
  preheader shape in this pipeline. Measured, it is byte-identical with the
  pass on and off -- LICM already hoists the unguarded load, so the pass has
  nothing left to unlock. The gate therefore asserts refusal for it rather
  than effect, which is the true behaviour.

* **RED-WIDEN-1 · widening reductions.** `int s; for (i) s += a[i];` with
  `a` of `short`/`unsigned char`: lccc stays scalar (3.0 / 5.0 insn/byte)
  where gcc does 0.28 / 0.53. Needs the detector to accept
  `accumulator_type != element_type` and a widen-then-fold body
  (`vpmovsxwd`/`vpmovzxbd` + `vpaddd`).


### ZERO-REM-1 · **LANDED 2026-09-28** — dead vectorizer remainder loops
The map vectorizer emitted its scalar mirror (the `N % W` tail loop, which
doubles as the runtime dependence guard's fallback) even when a constant trip
count made it unreachable: 17 instructions of guard, resume-index arithmetic,
materialised stream pointers and a dead scalar body **per vectorized loop**.
`transform_map_vector` now omits it when `N % packed_width == 0`,
`pattern.guarded_streams` is empty and the escaping counter can be re-pointed
at the trip count (materialised in the preheader with the mirror's own
`Cast`-to-`iv_ty` form, so it works in bare-`Value` use positions too).
Measured at `-O2` on upstream `main` `93f2a43b` (A/B against the kill
switch): `sha256_transform` **151 → 135** insns / 20 → 19 rrmov / 1 → 0
stkref, a 16-element `unsigned` copy 61 → 28, `shape_u32_exact` 39 → 18.
Pinned-oracle targets for the same function: GCC 16.2 = 142, Clang 23.1 =
129. Kill switch `CCC_NO_MAP_ZERO_REM=1`.
Evidence: [`engineering/evidence/ZERO-REM-1/README.md`](engineering/evidence/ZERO-REM-1/README.md);
gates `tests/regression/check_vec_dead_remainder.sh` +
`tests/regression/check_vec_remainder_shapes.py` +
`tests/regression/vec_dead_remainder.c` and the assembly-shape fixture
`tests/regression/vec_shapes/vec_dead_remainder_shapes.c` (a subdirectory, so
`run_regression.py`'s program corpus does not try to link it).
Not done: the reduction and stencil vectorizers build their own tails; the same
constant-trip-count reasoning applies there.

### SPILL-01 · **TOOL LANDED 2026-09-28** — causal stack-reference census
`ra_quality_census` counted stack references; nothing said *why* they exist.
`RegAllocResult` now publishes the allocator's own `eligible` set and
`src/backend/stack_layout/slot_census.rs` turns it into a per-slot cause
(`alloca`/`address`/`wide`/`spill`/`nongpr`/`temp`, opt-in `CCC_SLOT_CENSUS=1`);
`scripts/stack_census.py` joins it with the POST-PEEPHOLE assembly and
attributes every emitted stack reference (plus `csave`/`argout`/`incoming`).
Corpus gate met: **98.74 %** coverage (1251/1267) vs the 95 % criterion.
Finding that re-orders the queue: **44.4 % of stack references are
callee-save save/restore**, 23.8 % structural (alloca/address/wide/nongpr) and
only **10.7 % are spills** — and `sha256_transform`, the RA-PRESSURE-3 target,
has **zero** spill references.
Evidence: [`engineering/evidence/SPILL-01/README.md`](engineering/evidence/SPILL-01/README.md).

### DO-WHILE-BRANCH-1 · **NEW 2026-09-29** — bottom-tested backedges pay 4 instructions for one branch
A bottom-tested loop's exit condition is materialised as an `i1`, zero-extended
to `i32`, then `test`+`jne` — while the `cmp` that produced it sits in the same
block, unused as a branch:

    .LBB1:  addl $1, %esi
            cmpl %edi, %esi
            setl %r8b        <-- 3 instructions and 1 uop
            movzbl %r8b, %r8d    wasted per iteration
            testb %r8b, %r8b
            jne .LBB1

Measured scope (`-O2`, **no env vars**, counting `setCC` inside the loop body):
all six bottom-tested shapes tested show it — `do{}while` with `<`, `!=`, `<=`,
step 1 and step 2, returning the counter or a constant, and `for(;;){…break;}`.
The TOP-tested `while` and the do-while the vectorizer turns into
marching-pointer form both already emit `cmp` + `jCC` directly, so the machinery
exists and this is a missing case in one lowering path, not a missing
capability. 9 of the first 60 `tests/regression/*.c` emit at least one `setCC`.
NOT the same item as LOOP-PREHEADER-2's rotation note: `loop_rotate` is opt-in
(`CCC_LOOP_ROTATE=1`), and `CCC_DISABLE_PASSES=loop_rotate` leaves every count
above unchanged, so this is the default bottom-tested lowering, not rotation.
Reproducer: `int f(int n){int i=0; do{i++;}while(i<n); return i;}` — no
pointers, no `main`, no specialisation involved.
Done = the loop body contains no `setCC` and the backedge is a single `jCC`,
pinned by an assembly gate, with no instruction-count regression elsewhere.
Full analysis: `engineering/FOLLOWUP-2026-09-29-pr681-ci-red-audit-adjudication.md`.

### LOOP-PREHEADER-3 · **NEW 2026-09-29** — LICM's must-execute rule is stricter than it needs to be
`loop_preheader` makes a guarded loop's preheader dedicated, and LICM then
hoists the loop BOUND — but a load in the loop BODY is still refused, because
LICM requires the load's block to dominate *every* loop block, which only the
header does. Measured on the SQLite `if (p == 0) return;` shape: steady-state
memory operands per iteration go 2 -> 1 with the pass, not 1 -> 0
(`check_loop_preheader.sh` contract 2 pins exactly that delta).
The rule that is actually needed is weaker and still sound: a block *dominated
by the header* is entered only when the loop is entered, because the loop-exit
branch cannot be taken before the body's first instruction runs. "Must be the
header" is a special case of "must be dominated by the header".
This widens the gate that prevents the documented `sqlite3_get_auxdata` NULL
segfault, so it needs the SQLite fixture, the `20051215-1.c` guarded-deref
torture shape, and a differential sweep before it lands. Not a drive-by.
When it lands, tighten `check_loop_preheader.sh` contract 2 from `-ne 1` to
`-ne 0` — the gate was written so that this is a one-token edit.

### LOOP-PREHEADER-4 · **CLOSED 2026-09-29** — the pass's module doc motivated itself with a loop it does not fire on
*(numbered -4, not -2: LOOP-PREHEADER-2 is already taken by the `loop_rotate`
default-enable item in `engineering/journal/2026-09-W3.md`.)*
`src/passes/loop_preheader.rs` opened with `for (i = 0; i < n; i++) t += c[0];`
as the shape a dedicated preheader unlocks. Measured: the pass prints nothing
for that function, because its own profitability gate mirrors LICM's
must-execute rule and the load sits in the body, not the header — that spelling
is the `while_sum` shape, which the pass *refuses* on purpose.

Closed: the docstring now leads with the do-while (guard outside the loop, so
the load is in the header) and keeps the counted `for` as an explicit
counter-example, and both shapes are pinned by
`tests/regression/check_loop_preheader.sh` contracts 1 and 2, so the comment
cannot drift away from the behaviour again.

### WR-COND-RETEST · **NEW 2026-09-29** — second conditional store re-tests a live condition
`void wr_cond(volatile u32 *p, volatile u32 *q, int c){ *(c?p:q)=1; *(c?p:q)=2; }`
emits `testl %edx, %edx` twice — once for `cmovneq %rdi, %r8` and again for
`cmovneq %rsi, %rdi` — instead of reusing the first select's result. One
redundant `test` per pair of same-condition selects. Small; worth folding into whichever pass
DO-WHILE-BRANCH-1 ends up needing.

### RA-CSAVE-1 · **NEW 2026-09-28** — callee-save save/restore traffic
Opened from the SPILL-01 census: 562 of 1267 stack references (44.4 %) are
callee-saved registers being saved on entry and restored on exit, more than
spills and allocas combined. Every register taken from the callee-saved pool
costs two stack references per call. Before touching eviction policy: measure
how many of those registers are *used* after allocation (`[RA-STATS]
callee-homes=`) versus saved defensively, and whether the corpus's hot
functions would rather spill a caller-saved value. Done = a census-driven,
output-checked change that reduces `csave` references on the corpus without an
instruction-count regression, or a documented bound.

### RA-PRESSURE-3 · **RE-SCOPED 2026-09-28** — sha256 round loop
The historical framing ("land a generic phi-copy cycle resolver") is
obsolete: a general resolver already ships (Kahn decomposition in
`plan_edge_copies`, exhaustive semantic-equivalence test over EVERY
parallel-copy graph for n=2..5, default-on policy, and
`check_phi_acyclic_order.sh` pinning rotation < legacy statically).
Re-measured at `-O2` on `6f8ace9c`: `sha256_transform` is **148 insns /
14 rrmov / 10 stkref** (GCC 142/19/8, Clang 129/24/26) — not the
historical 218 insns / 64 spills, and lccc now has FEWER stack
references than Clang. The residual gap is instruction selection and
*physical* rotation, not copy resolution.
New sub-problems: (1) express the rotation as a register permutation
(rotate-by-immediate / `rorx`-class) rather than copies — check
`ui: alu/phi` admission, see `span_recurrence`; (2) 2×/4× unroll so the
rotation runs once per body; (3) close the 19-insn gap to Clang.
Negative space unchanged: the valve count-coupled supply is falsified
(W2 09-11); RA-06 intra-block splitting-measurement is recorded dead
(W1 09-05).

### RA-PRESSURE-4 · Aggregate/struct register pressure — `struct_copy` (1.15–1.45×)
156 insns vs clang 76 (**77 spills**). `CCC_NO_AGGREGATE_SPLIT` escape.
Done = ≤1.05× on Raptor-Lake report.

### PF-CHACHA-1 · chacha20 ARX schedule — remaining ICX gap (118 insns vs icx 74 @-O2, icx 63 @v3)
Refreshed 09-27 (oracle, static counts): PR #638's ARX work already took
the `-O2` body from the old 592 and the v3 body from 222 down to **118**
(oracle: lccc 118 < clang 176 < gcc16.2 180; icc 1104). The remaining gap
is ICX only (74 @`-O2`, 63 @v3): the v3 ARX-lane schedule (superword diag +
horizontal ARX; W2 09-08) plus ICX's tighter quarter-round folding at base.
Done = v3 closer to ICX's 63, `-O2` static ≤1.15× of ICX's 74 (runtime
parity on the author's box no longer counts as done — keep static and
runtime numbers separate).

### PF-SCHEDULER-1 · sha256 message-schedule vectorization
GCC vectorizes the sliding-window expansion even at -O2 (`m[i-2..i-16]`
SIGTA → SSE). Blocker: strided/sliding recurrence — unaligned vector
loads at constrained offsets + sliding-window SLP (`vec_arx` /
`arx_vectorize` first-light: `s10/next/solve/split` → v-slp flags).
Done = schedule vectorized at -O2, kernel ≤1.15×.

### PF-TLS-1 · TLS segment access — **DIRECT LOCAL-EXEC FORM LANDED (2026-09-28)**
Legal Local-Exec accesses now lower to ONE instruction,
`%fs:symbol@TPOFF(+N)`, instead of base materialization + access.
Measured at `-O2` on `6f8ace9c` → this tree (`ra_quality_census`):
`set_all` 54 → **36** insns, `sum_all` 41 → **28**, a 3-store `set`
12 → **6** (byte-identical to GCC), benchmark `tls_pass` 34 → **30**
(GCC 37, Clang 31), whole focused TU 123 → 91.
Legality rule (measured against GCC, not guessed): Local-Exec is a
link-time constant, so it is used for every executable — PIE included —
but **never** for `-shared`, and only for a symbol this module owns
(extern TLS keeps Initial-Exec, as GCC does). The backend therefore
carries a new `shared_lib` flag: `pic_mode` alone cannot tell `-fPIC`
from `-shared`, and the two have opposite legality here.
Two prerequisites were repaired first, both reproducible on the review
baseline: (1) the assembler dropped the relocation modifier for
`sym@MOD+N` (`R_X86_64_32S` against a symbol literally named
`sym@TPOFF` — a silent wrong address; lccc's relocations now match GNU
as 2.47 exactly); (2) `lccc -fPIC -shared` could not link ANY
`static __thread` ("R_X86_64_TPOFF32 ... can not be used when making a
shared object") because shared output used Local-Exec — it now uses
Initial-Exec, verified by a `dlopen` round-trip.
Runtime: **not established** — the scaled `tls_seg_access` screen
(`-DPASSES=20000000U`, 7 interleaved rounds, output identical to GCC)
has a per-binary spread of 0.67–0.94 s, so the before/after medians are
inside the noise. UNVERIFIED ON TARGET.
Evidence: [`engineering/evidence/PF-TLS-1/README.md`](engineering/evidence/PF-TLS-1/README.md);
gates `tests/regression/check_tls_model_selection.sh` +
`tests/regression/tls_local_exec_direct.c`.
Remaining: two of three `&tls_slots` computations still not CSE'd; a
variable-index TLS read still pays the base (unavoidable); `%gs:` and
i686 coverage not attempted.

### PF-CLS-1 · Byte-classifier chains (expat_xml_scan 1.31–1.37×)
**Re-measured 2026-09-28 at `-O2`: `expat_utf8_name_length` is 70 insns
vs gcc 79** — lccc is now statically AHEAD, so the 1.31–1.37× runtime
gap is pure branch/layout behaviour and instruction-count work here is
wasted effort. Use `scripts/callgrind_ab.py` (deterministic `Ir`,
I1/LL misses, branch mispredictions) or a target-CPU PMU run; this
host's wall-clock noise floor is ~15 %.
Historically: 79 vs gcc 78 insns — branch-prediction/scheduling-bound,
not insn count. Structural block: `a||b||c` last-member critical edge + shared
increment block starve if-conversion (two attempts reverted W1 09-01g).
**Split the critical edge on the last member first**; the counting
spelling `pred && n++` needs the same `Select`s as the boolean one.
Sub-items: (1) length-table load DCE; (2) char-class increment-starved
`vectorize` falling back to `if_convert` (1.17×): sequence-first
vectorization.

## Tier 2 — MachInst / isel

### MI-CLOBBER-1 · Clobber modelling, then lower `Call` (7.8 % of insns)
Correct-by-rejection today (the flush boundary *is* the clobber model).
`CallTyped` carries declared clobbers (W1 09-02e); extend per-instruction
clobbers before lowering `Call` — adding `Call` uses before that is a
miscompile.

### MI-XMM-1 · Vector/FP register class (1.3 %)
119 rejected `Store(float)`. MachInst models only the 16 GP families.

### MI-PARAM-1 · Remaining `ParamRef` (1.1 %)
No-code subset landed (963 → 96). Remainder is emissive: alloca-homed
params and stack-passed args; the fallback reads the parameter's
**incoming** register even when a caller-saved pre-store of a different
parameter aliased the name.

### MI-ROTATE-1 · Sub-word rotates
Native `RotateLeft/Right` landed (W2 09-07). Open: sub-word rotates
whose complements meet only at the narrow width (truncation-aware
pattern / a consuming `Cast(i32→u16)` rewrite). Count-staging polish is
cosmetic; do not chase.

### MI-ENCODE-1 · Encoding-level differential
Suite at 7 layers/36 tests incl. execution vs GAS 2.47; byte-level arm
via `scripts/insndiff.py` / `encoding_diff.py` + GAS 2.47 (INF-GAS-1).

## Tier 3 — verifier / infra

### FE-RELRO-1 · CLOSED — multi-level pointer const leaked into `.data.rel.ro` placement (2026-09-27)
The parser OR-ed `decl_flag::POINTER_CONST` across every star of a
declarator, so `short * const * gp` (outer object **writable**) was
classified read-only and `classify_global` routed it to `.data.rel.ro`;
RELRO page-protection then made the first run-time store SIGSEGV
(Csmith seed 20260928, differential vs GCC at -O0; wild store to the
globals page, gdb-confirmed read-only mapping at run time). Fix:
each star iteration now *clears* the flag, so only the identifier-
adjacent (outermost) level's const survives — which also fixes
cross-declarator leakage (`int * const a, *b;` made `b` read-only).
Regression: `tests/regression/pointer_const_multi_level_relro.c`
(SIGSEGVs on the pre-fix build; byte-compares vs GCC).
Do-not-reopen: the single-star `T *const p` case is
`pointer_const_data_rel_ro.c` (kernel zstd contract); the volatile
sibling (`short * volatile * gp` marks the declaration volatile —
conservative, safe, missed-opt only) is deliberately NOT changed in
the same commit; measure before touching it.

### VER-PHIARITY-1 · CLOSED — φ-arity contract is two-sided: reachable ⊆ named ⊆ static (2026-09-27, refined after corpus run)
phi elimination materializes one edge copy per (reachable pred,
incoming) pair and never visits unreachable blocks, so (a) a statically
present but dead predecessor legally has no φ incoming — the frontend
lowering leaves such dead edges routinely — and (b) an entry for a dead
predecessor is inert (the edge copy sits on a path that never runs;
switch/loop lowering produces these, e.g. switch_dispatch's 16-pred
join). The first cut of the reachable contract enforced EQUALITY with
the reachable set and false-positived on 15 corpus files, all of the
`extra [dead-pred]` class. Final contract, identical in BOTH checkers:
every REACHABLE predecessor must be named (missing = live-edge defect,
the miscompile class), and only real CFG predecessors may be named
(stray = carelessly retargeted edge); an entry for a dead predecessor
sits legally in the gap. GLA check 7 (src/backend/location_alloc/
verifier.rs) and the `CCC_VALIDATE_SSA` PHI-ARITY check (src/passes/
mod.rs) implement it with unit tests for both sides. Do not weaken the
reachable-preds coverage side: that is the real defect detector
(Csmith 20260981's compile abort). Also added a
`lowering:entry` validator checkpoint (at -O0 the optimizer loop is
empty, so without it a frontend IR defect was only visible at
`backend:pre-eliminate_phis`) and dump-before-validate ordering there.

### PHI-SINK-1 · CLOSED (superseding entry) — composed with #645; profitability = live phi home (2026-09-27, S66)
The unguarded sink regressed `spectral_norm` (289 > 287 budget): sinking the
accumulator's add across the body/latch boundary separated it from its mul
and lccc lost `vfmadd231sd`. Final rule: sink ONLY when the phi home is
live across the window — an instruction between definition and relay (same
block) or in the strictly-dominated region reads or writes the home (the
rotation's `f <- e` copy). Dead-home accumulator shapes are refused; the
allocator coalesces them anyway and the fold would only distance the
computation from its operands. Both behaviors unit-pinned
(`computed_incoming_folds_to_its_copy_slot`, `dead_home_computed_incoming_is_not_sunk`,
`fold_sinks_definition_without_touching_readers` — rewritten after it was
found to pass vacuously under refusal). Composed with #645 (Agent B's
rotation lags + residency guard + lea addend fold — audit:
`engineering/AUDIT-PR645-reassoc-latency.md`): rot 55/3 → 53/3, sha256
144→142, corpus −5 insns/−5 rrmov with 183/186 functions identical, full
lib 3621/0. Gate ratchet: escape-off census pinned to the exact composed
shape (k_i ≤ 53 / k_s ≤ 3); the beat-gcc and beat-legacy-by-10 contract
unchanged. Do-not-reopen: the predicate gates profitability only —
correctness always stays with the RA's disjoint-interval coalescing rule.

### CC-O0CALL-1 · **CLOSED 2026-09-28** — secondary-cache (`%rcx`) clobber by GP argument 4
The previously proposed cause (`-O0` accumulator-address loads trusting a
non-SSA cache hit) is **falsified**: applying that guard leaves Csmith
20260945 SIGSEGVing, and its regression test passes on the broken
baseline, so it does not discriminate.
Actual cause: `%rcx` is both the secondary value cache and SysV's fourth
GP argument register. Argument 4 staged a scalar into `%rcx` without
invalidating the cached pointer identity, so the next (aggregate)
argument rematerialized the "pointer" from `%rcx` and dereferenced the
scalar that had just been written — `movq %rcx,%rax; movq (%rax),%r8`
with a NULL base. Independent of the peephole and of GLA (also fails
with `CCC_NO_PEEPHOLE=1` and `CCC_RA_GLOBAL_LOCATION=0`).
Fix: invalidate the secondary cache after a GP argument writes ABI slot
3 and before the next argument is staged, for every classification that
can write it (scalar, i128/i64 register pairs, by-value aggregates, both
mixed struct classes); the next-iteration placement also covers the
early-`continue` staging paths.
Evidence: reduced reproducer
`tests/regression/call_secondary_cache_clobber.c` (8 ABI shapes), gate
`tests/regression/check_call_secondary_cache.sh` (20 output-checked
arms: `-O0..-O3/-Os` × {default, no-peephole, no-GLA, both}), the full
Csmith seed 20260945 now prints `checksum = 39DEBCF9` / exit 0 like GCC,
and GCC `-fsanitize=address,undefined` is clean on the reduced case.
Historical reproducer kept:
`artifacts/repros/miscompile_csmith_20260945_-O0.c`.

### INF-HARNESS-1 · Subtract startup / scale short benchmarks
lz4's 1.94× hid a 10× work gap (~2 ms fixed cost in a 7 ms measure).
Report rule: no benchmark with fixed cost > 10 % of measured time
reported bare; short kernels scaled or decomposed.

### INF-BENCHGATE-1 · Benchmark output gate unconditional in CI
`scripts/check_benchmark_outputs.sh` runs every `tests/benchmark/
programs/*.c` at all four levels (~4 min, no timing) — a miscompile
once passed all regression tests because those kernels were only ever
built by the timing harness.

### INF-FRESHCLONE-1 · CI builds from a fresh clone
Historical failure mode: files missing from a commit while the author's
tree was fine.

### INF-GAS-1 · GAS 2.47 re-provision per session
`scripts/ensure_gas_247.sh` — caches/snapshots do not persist it;
`/usr/bin/as` 2.44 is **not** an oracle.

### INF-LINK-1 · Linker oracle pins
lld 23.1 (`release/23.x`), mold 2.42.1, bfd 2.47 — verify versions
before trusting a prebuilt (W3: a stale `wild` 0.7.0 produced two false
lccc failures). Setup: `tools/linker/setup_oracles.sh`,
`tests/linker/setup_oracles.sh`.

---

### PERF-1 · Byte-width and masked sum reductions are not vectorized

Isolated with a four-case experiment (`-O3 -march=x86-64-v3`, vector
instructions emitted per loop):

| shape | LCCC | GCC 16.2 |
|---|---:|---:|
| `c += p[i]`, `int *` | 18 | 25 |
| `c += p[i]`, `signed char *` | **2** | 43 |
| `if (p[i]) c++`, `int *` | **1** | 22 |
| `if (p[i]) c++`, `signed char *` | **1** | 46 |

So plain I32 sum reductions vectorize and nothing else does — neither byte
width nor masked reduction, at any width.

Consequence: `sieve` runs 1.121x slower than GCC, and LCCC emits **zero**
vector instructions for the whole file. GCC vectorizes the prime-counting
loop as `vpcmpeqb` -> `vpmovsxbw` -> `vpsubd`.

Blocker: `src/ir/intrinsics.rs` has only I32->I64 widening
(`VecWidenAddI32x4ToI64x2`, `VecLoadWidenI32ToI64x2`). There is no I8->I32
widening op in the IR or the x86 backend, so this needs a new intrinsic, its
`vpmovsxbwd`+`vpaddd` lowering, vectorizer pattern matching for I8 element
types, and remainder handling — three layers, and a miscompile if rushed.
Start from the four-case table, not from the symptom.

### PERF-2 · No loop interchange pass (matmul 1.399x, worst kernel)

`grep -rn "interchange" src/passes/` finds nothing. GCC rewrites
`C[i][j] += A[i][k]*B[k][j]` from i,k,j into i,j,k so `C[i][j]` is contiguous
and `A[i][k]` is broadcast, and unrolls k by 2. LCCC keeps source order and
stores the accumulator to memory on every k iteration.

`matmul` is the worst ratio in the benchmark corpus at 1.399x. Interchange
needs dependence analysis to prove legality plus index remapping to rewrite
the nest — a project, not a patch.

### PERF-3 · Register-move gap in zlib_ng_adler32 is code size, not speed

LCCC emits 39 reg->reg moves against GCC's 12, which looks alarming. Measured
per block: the moves are spread across cold blocks, and the hot inner loop
(`.LBB13`, 29 instructions) contains only **2** of them. The kernel already
runs **faster** than GCC (0.930x). Recorded so the next engineer does not
spend a day on it: it is a size defect on a kernel that already wins.

## Closed this cycle (do not re-open without new evidence)
<!-- durable here for grep-ability; narrative in engineering/journal/2026-09-W2.md and 2026-09-W3.md -->

VER-DOM-1 (def-dominates-use in the IR verifier — landed as
`src/passes/verify.rs` checks 7 & 8: SSA single definition and
def-dominates-use over an interval-encoded dominator tree, with the
diamond/loop/terminator test family in `verify/tests.rs`; the 2026-09-27
triage found this item stale — the dominance check has been in place
since the IR-verifier birth arc);

`__builtin_memcpy` → native Memcpy + the two latent aliasing fixes it
exposed; chacha20/ARX 10.01× → ~1.05×; `iv_widen` constant-scale firing
(sieve −21.6 %); order-preserving block layout; `CCC_VERIFY_IR` 0
violations/6 levels; machine-level loop inversion (memchr → parity);
S05 inline-single-site-static; S06 cmp-branch-fusion (4 holes, 16
tests); S07 ifcombine profitability (+4.7 % lz4); S09 vectorizer
cond-store use-without-def; S10 vectorizer IV-live-out (`wsum=511`);
bool-pair tail-jmp; loop-memset (lz4 −1.75 % Ir); native rotates; imm32
bit-pattern hoisting; duplicate `GlobalAddr` merge (expat −23.3 %,
lz4 −15.3 %); RA web-wide in-loop-use supply (+3.63/+4.33 % sha256,A/B)
+ boolean-only valve supply; phi acyclic copy order (opt-in
`CCC_PHI_ACYCLIC_ORDER=1`); SROA split `0.0`/`-0.0` bit-exact zero
(CG-07); epilogue-suffix sharing (CG-09); OP-42 transactional sinking;
RA mode-6 stays opt-in (RA-28/28b); TLS CSE merge (3→2).
