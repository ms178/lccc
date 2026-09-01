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

### IV-DERIV-1 · Widen constant-offset derivatives of the induction variable
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

### RA-PRESSURE-1 · Register pressure and copy webs — 1.43–1.49×
*Evidence:* `spectral_norm` 1.491, `struct_copy` 1.463, `arith_loop` 1.430.

Lifetime demotion spills whole live ranges instead of splitting them.
**Done when:** live-range splitting at the demotion point; `arith_loop` ≤1.15×.

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
