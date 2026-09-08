# LCCC Engineering Backlog

Last rebuilt: **2026-09-01**, against `main` @ `be93d266`, from the full 33-kernel
benchmark run (9 paired rounds, all 33 checksums byte-identical to GCC 14.2.0)
and the instruction-selection census (`CCC_ISEL_STATS=1`, 562 files).

Ranked by **measured impact**. Every entry states its evidence, the specific
blocker, and what "done" means. An item with no reproducer does not belong
here. Several entries are deliberately *not* implemented because the safe
version is not yet reachable — those say so rather than being quietly dropped.

**Attribution rule.** A ratio is only meaningful when both arms were measured
in the same window. Never compare ratios across reports to claim a regression;
use a paired same-window A/B with kill switches. Applying that rule this cycle
showed two of three previously "reported" regressions did not exist and one
was an improvement.

---

## Tier 1 — measured, reproducible, largest first

### IV-DERIV-1 · Widen the induction variable's derived closure
*Status:* designed, implemented, measured, **reverted for a miscompile**. Full
record in `engineering/FOLLOWUP-2026-09-01l-iv-closure-attempt.md`.

*Measured prize (same-window A/B, before revert):* `sieve` **−26.5%**,
`loop_patterns` **−6.5%**, `tls_seg_access` **−5.7%** (the worst kernel in the
corpus), `histogram` −2.8%; `nbody` +3.2%.

*Why it failed:* the closure collector admitted ops whose operand was not in
the closure -- widening phi `v119` pulled in `And(v81, 31)` and
`Mul(v76, 104729)` -- so the interval proof was attached to values it did not
describe. Two other soundness bugs were found and fixed on the way and are
documented: the `Or` range bound `hi | c` is not an upper bound (counterexample
`x ∈ [1,2]`, `c = 2`), and the trip bound must come from the comparison the
loop HEADER branches on, not from any comparison mentioning the counter.

**Done when:** membership records, for each admitted member, which closure
value it derives from, with a `debug_assert!` that the operand really is that
value; and `scripts/check_benchmark_outputs.sh` stays green.

### IV-DERIV-2 · (superseded heading kept for reference)
*Evidence:* `tls_seg_access` carries two `movslq` per iteration, one on the
loop-carried path. Widening the counter itself now works for every element
width (see closed items), but this loop's counter has genuine *arithmetic*
uses -- `s[i-1]` and `s[(i&7)+1]` -- and `analyze_iv_uses` bails on any
non-cast use.

`i-1`, `i+1` and `i&mask` feeding addressing are the stencil/recurrence shape
and are everywhere. Widening them is value-preserving for a counted loop, but
proving it in general needs range information, so the safe subset is:
constant-offset derivatives of an IV whose bounds are known constants.

**Done when:** `tls_seg_access`'s inner loop carries no `movslq`.

### PF-TLS-1 · TLS segment access — 2.19× slower
*Evidence:* `tls_seg_access` 24.6 ms vs GCC 11.4 ms.

LCCC stages the thread pointer (`mov %fs:0x0,%rax`) in the prologue of every
TLS-using function; GCC addresses TLS directly with `%fs:offset` and
`%fs:(%idx,scale)` operands. The link-time-constant-offset path does not cover
dynamic offset forms, and two of three `&tls_slots` computations per function
are duplicate and never CSE'd.

**Done when:** dynamic-offset TLS reads emit a single `%fs:` operand with no
thread-pointer staging; `tls_seg_access` ≤1.1×.

### PF-CLS-1 · Byte-classifier chains — 2.09× slower
*Evidence:* `expat_xml_scan` 86.9 ms vs 41.5 ms. Reproducers:
`tests/bench/k_namechars.c` (counting form) beside `tests/bench/k_classify.c`
(boolean form) — deliberately bracketing the defect.

`if (pred) n++` over an `a || b || c` predicate emits an **eleven-branch chain
per byte**. GCC vectorises it; ICX uses a binary search plus adjacent-constant
folding (`ch=='-'||ch=='.'` → `addb $-45; cmpb $1`).

The pipeline is `if_convert` → `range_fold` → `set_membership`, starved at the
**first** link. The boolean-returning spelling of the same predicate gets the
full treatment (range folds, then the `[a-z]`+`[A-Z]` case-fold merge on
`c & ~32`); the counting spelling produces no `Select`s because every test's
hit edge funnels through one shared increment block before the join, so the
join phi has two incomings no matter how many tests there are.

**Two attempts are documented, both instructive:**
- Normalizing the funnel (`hoist_merge` + generalizing `set_membership` beyond
  `Const(1)` hit values) *worked* — one phi incoming per test, 11 branches → 10
  — but **miscompiled**. The last member of any `a || b || c` chain branches to
  the hit block *and* falls through to the join, so retargeting gives one
  predecessor two edges with different values. That critical edge is
  structural, not incidental. Reverted; see `FOLLOWUP-2026-09-01g`.
- Even with the phi normalized, `range_fold` still would not fire: the `&&`
  pairs remain branches, so members parse as `Skip`, not `Range`.

**Done when:** the pure single-entry/single-exit test region is if-converted to
predicate arithmetic (`reach[B] = OR over preds of reach[P] AND edge_cond`),
after which `range_fold` and `set_membership` work unmodified. **Split the
critical edge on the last member first.**

### RA-PRESSURE-1 · Register pressure and copy webs — 1.43–1.49× (chacha20: 2.85×)
*Evidence:* `spectral_norm` 1.491, `struct_copy` 1.463, `arith_loop` 1.430,
`chacha20_block` 2.85× (2026-09-07, after the rotate fold — what remains is
pure register-allocation staging: 178 movs in the hot function vs GCC's 65).

Lifetime demotion spills whole live ranges instead of splitting them.

**chacha20 mechanism (diagnosed 2026-09-07):** the loop-spanning x-value
coalesce webs (preheader copy + header phi + latch update, 16 of them)
occupy every register the pool offers; every quarter-round temp is
single-use, and the spiller prices an interval at `uses.len()`, so cost 1
always loses to a phi web — all ~60 in-loop temporaries stack-homed and
the loop body runs through `%eax` staging. GCC spills a few x-values at
round boundaries and keeps the temps in registers.

**2026-09-07 progress — loop-span admission cap (implemented, default
OFF):** liveness exports per-loop linearized `[header, latch]` extents;
`mark_loop_spanning` measures each loop's block-local PEAK concurrency
and total weighted cost; the allocator caps loop-spanning admission at
`pool − peak` (knob `CCC_RA_LOOP_SPAN_RESERVE`, 0 = byte-identical old
behaviour) with a pigeonhole gate, a cost bar, and a recurrence guard
(once-per-pass spans must not be demoted — arith_loop lost 9.7% to
exactly that despite winning the frequency count). Measured in-window:
chacha20 +29%, sha256 +27% armed at 3. The VM throttled 60× later in the
session (a fixed binary: 0.809 s → 49.8 s), so the default-on census is
open — see `engineering/FOLLOWUP-2026-09-07-ARX-CHACHA-RA.md` §3 for the
full record, including why the GEP-base/coalesce-web priority boosts
must stay OUT of the main waves (they reorder the scan: sha256 −56%).

**2026-09-08 — RA-PRESSURE-2 landed (default ON, knob-independent):**
`mark_loop_spanning` now computes a web-wide `span_recurrence` flag (a
non-phi member's def consuming another member = the carried chain itself:
arith_loop's accumulators, sha256's a..h schedule) and
`span_exposed_uses` (in-loop reads whose consumer is latency-exposed vs
folded addressing). Two always-on mechanisms replace the default-cap
question: (1) knob-independent invariant demotion — spans with no in-loop
read demote at admission under the same pigeonhole gate, gated on the
profile having been measured (`span_marked`); (2) the span-pressure
valve — a non-span incoming at an exhausted pool evicts the steal-safe,
non-recurrence, ≤2-future-use active span (Braun–Hack MIN). Plus the
machinst window pool gains rax/rcx (interference-tracked per window;
`Raw` blocks the pool for div). Result: chacha20 220→155-insn loop,
0.327→0.258 s (GCC -O2 0.240, was 2.85× → now 1.07×); corpus min/5
paired: sha256 −4.9%, fannkuch −4.2%, spectral −3.3%, crc32 −2.0%,
adler32 −1.1%, arith −0.7%, memcmp/expat/matmul flat, nbody +1%,
lz4 unmeasurable (6–12 ms bimodal in this sandbox; identical asm
through GCC). The `CCC_RA_LOOP_SPAN_RESERVE` knob remains (now with the
recurrence + exposed-use ceilings baked into `worth_capping`).

**Done when:** ultimately live-range splitting at the demotion point
(RA-06 `split_high_pressure_ranges` exists but measured a net LOSS on
chacha as-is); `arith_loop` ≤1.15×; beat-ICX on chacha (vectorized QR,
see RA-PRESSURE-3 below) — the remaining 7% to GCC and the ICX gap are
loop-carried x-value forwarding latency, not instruction count.

### PF-ADLER-1 · Accumulator recurrence — 1.24×
*Evidence:* `zlib_ng_adler32` 50.2 ms vs GCC 39.6 ms. Oracle at
`-O3 -march=x86-64-v3`: lccc **119** instructions, GCC 105, ICX 93, Clang 87,
**ICC 76** — lccc is last on this kernel.

The DO8 body is eight dependent `s1 += *buf++; s2 += s1;` pairs. ICC and Clang
break the recurrence; lccc does not. Now that block layout no longer distorts
this kernel, this is a clean SLP/reassociation target rather than an artifact.

### OP-VEC-1 · Non-reduction FP vectorization — 1.27–1.49×
`spectral_norm`, `nbody`: needs multi-store scatter and computed-invariant dot
analysis.

---

## Tier 2 — MachInst / instruction selection

Coverage **85.1%** corpus-wide (was 53.9%). A regression test fails if any
class the layer owns drops back out — the fallback to text emission is silent,
and a merge has already silently reverted one such fix.

### MI-CLOBBER-1 · Clobber modelling, then `Call` — 7.8% of instructions
**Currently correct, not defective.** `MachInst::Call` emits a bare
`call target` and the layer has **no clobber modelling**; what keeps
caller-saved values sound is precisely the rejection, because returning
`false` flushes the buffer and emits the run *before* the call. The flush
boundary **is** the clobber model.

**Done when:** MachInst carries a per-instruction clobber set, the allocator
honours it, and `Call` lowers. Adding `Call` uses before that is a miscompile.

### MI-XMM-1 · Vector/FP register class — 1.3%
119 rejected `Store(float)`. MachInst models only the 16 GP families.

### MI-PARAM-1 · Remaining `ParamRef` cases — 1.1%
The provably-no-code subset already lowers (963 → 96 rejections). What is left
is emissive: alloca-homed parameters and stack-passed arguments. Any
replacement must preserve the pinned rule that the fallback reads the
parameter's **incoming** register even when a caller-saved pre-store of a
*different* parameter aliased that register name.

### MI-ROTATE-1 · Rotate idiom follow-ups
Upstream #440 landed `IrBinOp::RotateLeft/RotateRight` (variable amounts,
all four backends, MachInst path) and 2026-09-07 fixed its cast-peeling
soundness holes — three live -O2 miscompiles, locked by
`tests/regression/bit_idiom_soundness.c`. Still open:
1. **Sub-word rotates whose complements meet only at the narrow width.**
   `(x16 << 8) | (x16 >> 8)` arrives promoted to i32 with counts 8+8 —
   not complementary at 32 — and needs the truncation-aware pattern
   (match the consuming `Cast(i32→u16)`, rewrite to a narrow rotate,
   DCE the or-chain). The complement-at-promoted-width shape
   `(w << 8) | (w >> 24)` already folds correctly as a u32 rotate of the
   extended value.
2. **Rotate-count staging polish**: the variable form materialises the
   count as `movslq %esi,%rdx; mov %edx,%ecx` — harmless (count consumed
   mod W) but sloppy.

### MI-ENCODE-1 · Encoding-level differential
The suite has seven layers and 36 tests, including **execution** against GAS
2.47. Comparing emitted *bytes* against GAS's own encoding would cover the
cases execution does not. `scripts/insndiff.py`/`encdiff.py` already exist.

---

## Tier 3 — verifier and infrastructure

### VER-DOM-1 · Def-dominates-use in the IR verifier
Six structural properties, 3372 configurations, six optimisation levels, zero
violations — but none of them is dominance. That is how an SSA violation once
shipped past every gate. `verify.rs` already computes reachability;
Cooper-Harvey-Kennedy dominators over RPO is the next step.

### INF-BENCHGATE-1 · Run the benchmark output gate in CI
`scripts/check_benchmark_outputs.sh` (new) compiles, runs and oracle-diffs
every `tests/benchmark/programs/*.c` at -O0/-O1/-O2/-O3 in ~4 minutes with no
timing. It exists because a miscompile passed all 563 regression tests: those
kernels were only ever built by the timing harness, so nobody ran them during
development. It should be unconditional in CI.

### INF-FRESHCLONE-1 · CI must build from a fresh clone
`main` @ `7a6eb81d` did not build: commit `9a2ef83a` declared
`pub(crate) mod decimal;` but `src/common/decimal.rs` was never committed.
Reverted upstream. This is the **second** time a file was missing from a commit
while the author's tree was fine (the first was `scripts/bench_kernels.py`,
swallowed by a `bench_*` ignore rule). A CI job that clones fresh and builds
would have caught both.

### INF-GAS-1 · GAS 2.47 must be re-provisioned per session
`.cache` is excluded from snapshots, so `scripts/ensure_gas_247.sh` must be
re-run after every environment wipe. The MachInst differential says so loudly
rather than silently degrading to the system 2.44.

### INF-LINK-1 · Linker oracle
Honour the pinned toolchain: lld 23.1, mold 2.42 (X86+i686-only preset),
bfd 2.47.

---

## Closed this cycle

| Item | Outcome |
|---|---|
| **chacha20 / ARX pipelines** | **10.014× → 2.85× vs GCC.** Stacked fixes: (1) the aggregate-SROA split ran *before* constant folding and copy propagation, so unrolled GEP offsets were still value *names* and every access looked variable — the split modeled nothing and the whole `u32 x[16]` state lived in stack slots. Fold+copyprop before the split, and unroll at `-O2`. (2) Native rotates: upstream #440 landed `IrBinOp::RotateLeft/RotateRight` (variable amounts, all backends) — merged **without a single test run**, and this session found and fixed three live cast-peeling miscompiles in it (rotate zext/sext halves; SWAR popcount and CLZ chains through truncating casts), plus the matcher ordering that keeps bswap-network recognition ahead of the rotate fold. 32 shift/or pairs became 32 `rol`, matching GCC's rotate count; `sha256_transform` rides the same fixes (2.757× → 2.03×). (3) Paired old-vs-new A/B: no other benchmark moved (matmul 0.995, lz4 1.000, bitops 1.001, nbody 0.977) |
| **IV widening** | Fired **only for byte arrays**: the addressing analysis accepted `Cast -> GEP` but not `Cast -> Shl(const) -> GEP`, which is the scaling chain for every wider element type. Now transparent to constant scales (`Shl`/`Mul`); variable scales deliberately excluded. Same-window A/B: `sieve` **−21.6%**, `nbody` −3.0%, plus small gains on `arith_loop`/`sqlite_varint`; also unblocks vectorization of int/long reductions |
| **Block layout** | RPO linearization cost **19%** on adler32 (55.8 vs 46.9 ms) by discarding the order earlier passes produced. Now starts from the existing order and fixes only contiguity: adler32 −15.6%, sqlite_varint −4.4%, memchr's 1.39×-vs-GCC win preserved. The pass had carried this cost since long before it was loop-aware |
| IR structural violations | 396 → **0** configs at all six opt levels; `CCC_VERIFY_IR` a permanent suite gate |
| Machine-level loop inversion | `memchr` −49.7% → parity with GCC; placed **after** phi elimination, where rotation is pure duplication instead of phi surgery |
| Vectorized loop counter | Silent miscompile (528/497 vs 504 from GCC, Clang, ICC and ICX alike); fixed at **zero instruction cost** by rewiring escaping uses to the remainder loop's element counter — 4.5× faster than the bail-out first shipped |
| Escaping-IV widening | `movslq` off the loop-carried path in gzip's `longest_match` |
| `range_fold` | Never fired on its own headline idiom: the boolean-widening allow-list stopped at `I32` while the frontend emits `U8 → I64`. Classifier −51% instructions |
| Register-copy folding | Rebuilt at all four widths after being cut at 0.03%; **12× more effective** once 64-bit address operands were allowed |
| MachInst | 53.9% → **85.1%** coverage; two real emitter bugs found by the GAS differential (`shlq %dl` for 8/16-bit shifts, unstaged wide immediates) and one by the fuzzer (`imulb` does not exist) |

---

## Method notes that keep paying off

- **Never attribute across runs.** Kill-switch A/B in one window, or do not
  make the claim.
- **Check what a switch actually disables.** `CCC_NO_BLOCK_RELAYOUT` turns off
  a pass that long predates the part I wrote; two hypotheses were spent inside
  my own code before comparing against the right baseline.
- **A pass can be correct, tested, and still the wrong transform.** RPO
  linearization was not wrong — it optimized the wrong objective, and nothing
  in the suite could see it because every output stayed byte-identical.
- **Hand-edit the assembly and time it before writing the pass.** Four variants
  of `k_memchr` showed the top to-do item was worth 0.01% and the one below it
  50%.
- **Know the noise floor** (~3.4% here, from code alignment alone).
- **Negative controls earn their keep**: two real bugs in copy-fold and one in
  the vectorizer were caught by tests asserting a transform does *not* fire.
