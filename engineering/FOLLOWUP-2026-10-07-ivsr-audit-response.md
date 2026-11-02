# 2026-10-07 — response to the audit of PR #774 (IVSR review-hardening round)

Base: `9d85b134` (upstream `main`, the merge of PR #774; my delivered tree
`e4eba4ef09f5f365145ae36ef86a871838f4d299` landed byte-identically as `bb492648`).
Host: 2-core Xeon VM, Debian 13, GCC 14.2, binutils 2.44, Valgrind 3.24.0, 8 GiB
swap. **Not** the i7-14700KF target, and no PMU: every number below is a VM
number, and none of them is a cycle, uop or speedup claim.

The audit under review ran no compiles. Every one of its nine findings was
reproduced or refuted against the tree before anything was changed, and two of
them turned out to be **more** right than the audit claimed. Nothing here is
accepted on authority.

---

## 1. Verdict on the nine findings

| # | audit severity | verdict | what changed |
|---|---|---|---|
| F1 | low | **VALID — my shipped claim was false** | §2. `FOLLOWUP-2026-10-07-ivsr-review-hardening.md` §4.1 corrected in place; `backlog.md` PERF-4 rewritten around the measured mechanism |
| F2 | low | **VALID, and incomplete** | §4. `UnsignedIvBound::exact` → `bounded`, redefined as "`hi` is the exact body maximum"; the descending ceiling now follows the init whether or not the limit is a literal |
| F3 | trivia | **VALID** | §5. `ENV_READ_BUDGET` 155 → 153, with the measured cause recorded in the ratchet's own history |
| F4 | low | **VALID — the doc said the opposite of the code** | §5. `is_used_as_address`'s comment rewritten around the A3 measurement |
| F5 | trivia | **VALID** | §5. census command made self-consistent: it now filters its own line and reproduces 18 sites / 10 files |
| F6 | trivia | **VALID** | §5. `const_in_iv_domain`'s "single definition" claim scoped to narrow IV domains, with the `I128` truncation named |
| F7 | **medium** | **VALID — the most consequential finding, proven by revert** | §3. New end-to-end fixture where the numeric certificate is the operative proof, plus the corrected claim in the old one |
| F8 | low | **VALID** | §5. evidence `README.md` 1232/two corpora → 1480/three, with the per-corpus rows |
| F9 | trivia | **VALID** | §5. `backlog.md` test list corrected: three tests → four, with the rename recorded |

Nine for nine. The audit's acceptance gate (new unit tests pass,
`check_ivsr_domains.sh` green, A/B unchanged, doc rows updated) is met in §7.

### 1.1 Where the audit stopped short

Being critical cuts both ways, so here is what the audit did **not** find:

* **F7's proposed fixture would not have worked.** W1 asked for "a narrow
  collected `mul_ty` fixture". The obvious candidate already existed — the
  `narrow_offset` arm of `descending_address_add_loop`, which scales the index
  inside the U32 ring — and it is useless as a certificate fixture, because
  `find_derived_exprs` does not collect `(I64)(i << 2)` at all: the scaled value
  reaches the address through a widening `Cast`, and neither `by_mul_dest` nor
  `find_affine` sees through it. That arm fires in **neither** direction (measured
  in the previous round, pinned by two assertions), so it can carry no
  certificate. The operative shape needs a `GetElementPtr` whose offset *is* the
  narrow multiply (§3).
* **F1's probe list was too short.** It suggested three spellings. Sixteen found a
  second defect the audit never mentions — `find_basic_ivs` records a modular
  decrement as `step = +2^32 - 1` — a third independent gate (no derived
  expression is collected for any descending probe), and a *different* reason the
  `u16`/`u8` spellings fail (integer promotion plus a truncate back, not the
  opcode) (§2).
* **F2's rename was right, its semantics were not finished.** "Flip `exact` to
  follow the init" is still wrong for an init at or below the limit: the body is
  empty, so there is no body maximum to be exact about. `bounded` is
  `init >= floor`, not "the init is a literal" (§4).
* **Neither the audit nor I had noticed the loose-bound opportunity.** An
  ascending IV with a loop-invariant limit gets `hi = type max`, `bounded = false`
  — sound but too weak to certify anything at `stride >= 2`. At `stride == 1`
  (byte walks) it *would* certify, so tightening that arm is a real optimization.
  It is deliberately **not** taken in this round: it would arm recurrences in
  common code, which makes it a perf change needing its own A/B and Callgrind
  round rather than a side-effect of a rename. Filed in §6.

---

## 2. F1 — what descending loops actually do (16 programs, measured)

The claim under audit, from the previous round's §4.1: *"No phi is ever
recognised, so no descending loop in any C program gets an IV recurrence."* It was
generalised from one spelling (`i-- > 0`). Probe: one function per spelling,
`-O2`, `CCC_IVSR_DEBUG=1` for the pass trace and `CCC_DUMP_IR=1` for the
recurrence.

`LCCC=target/fastbuild/lccc bash engineering/evidence/2026-10-07-ivsr-audit-response/descending-spelling-probe.sh`
(raw output beside it in `descending-spelling-probe.txt`). `T` is the counter and
array-element type; the accumulator is deliberately wider so it introduces no
wraparound of its own:

| # | spelling (`s = 0; for (…) s += p[i];`) | `BasicIV`? | constant-step recurrence | header test | derived exprs |
|---|---|---|---|---|---|
| s01 | `for (T i = n; i-- > 0;)`, `T = u32` | no | `Sub Const(1) U32` | `Ne U32` | – |
| s02 | `for (T i = n; i > 0; i -= 1)`, `u32` | no | `Sub Const(1) U32` | `Ne U32` | – |
| s03 | `i += -1`, `u32` | **yes** | `Add Const(I64(4294967295)) U32` | `Ne U32` | none |
| s04 | `i = i + -1`, `u32` | **yes** | `Add Const(I64(4294967295)) U32` | `Ne U32` | none |
| s05 | `i = i + (u32)-1` | **yes** | `Add Const(I64(4294967295)) U32` | `Ne U32` | none |
| s06 | `i = i + 0xFFFFFFFFu` | **yes** | `Add Const(I64(4294967295)) U32` | `Ne U32` | none |
| s07 | `i = i + (0u - 1u)` | **yes** | `Add Const(I64(4294967295)) U32` | `Ne U32` | none |
| s08 | `i += -(u32)1` | **yes** | `Add Const(I64(4294967295)) U32` | `Ne U32` | none |
| s09 | `for (u32 i = 64; i > 0; i += -1)` (literal init) | **yes** | `Add Const(I64(4294967295)) U32` | `Ne U32` | none |
| s10 | `for (i32 i = n; i-- > 0;)` | no | `Sub Const(1) I32` | `Sgt I32` | – |
| s11 | `for (i32 i = n; i > 0; i += -1)` | **yes** | `Add Const(I64(-1)) I64` (promoted) | `Sgt I64` | none |
| s12 | `for (u64 i = n; i > 0; i += (u64)-1)` | **yes** | `Add Const(I64(-1)) U64` | `Ne U64` | none |
| s13 | `for (u16 i = n; i > 0; i += (u16)-1)` | no | `Add Const(I32(65535)) I32` (promoted, truncated back into the phi) | `Sgt I32` | – |
| s14 | `for (u8 i = n; i > 0; i += (u8)-1)` | no | `Add Const(I32(255)) I32` (same) | `Sgt I32` | – |
| s15 | `u64` backwards walk, `s += p[i - 1]` | **yes** | `Add Const(I64(-1)) U64` | `Ne U64` | none |
| s16 | `u8` backwards byte walk, stride 1 | no | `Add Const(I32(255)) I32` (promoted) | `Sgt I32` | – |

So the audit was right: **descending `BasicIV`s form.** Ten of the sixteen
spellings produce one, and the reason the original claim looked true is that the
two most idiomatic spellings (`i--`, `i -= 1`) are emitted as `Sub`, which
`find_basic_ivs` does not match. Note what the `RECURRENCE` column says about the
*narrow unsigned* rows: the addend is `Const(I64(4294967295))`, never
`Const(-1)`.

### 2.1 Why A1's descending arm is still unreachable — now measured, not assumed

The descending arm of `unsigned_iv_bound` needs three things at once: a **narrow
unsigned** IV (`iv.ty.size() < target_ptr_size()` and `is_unsigned()`, which is
the caller's gate), `step == -1`, and a strict `Ugt`/`Ult` exit test. No probe
produced that combination, for two independent fail-closed reasons:

1. **The step is read from the raw carrier.** For a narrow unsigned IV the
   frontend zero-extends the decrement addend, so the IR holds
   `Const(I64(4294967295))` and `find_basic_ivs` records
   `step = +4294967295` — a huge *positive* increment, not `-1`. The polarity
   gate admits only `(Ult, +1)` and `(Ugt, -1)`, so the bound is refused.
2. **`i > 0` on an unsigned counter canonicalises to `Ne`**, which the same gate
   refuses.

The pointer-width and signed descending IVs that *do* carry `step = -1` never
reach the arm either: `modular_iv` is false for `U64`, and `is_unsigned()` is
false for the promoted `I64` — in both cases the caller passes `None` and the
product check exempts the ring.

### 2.2 The defect the audit did not report: a misrecorded step

`step = +2^32 - 1` for what is really `-1` is a genuine misrecording, and it is
latent only by accident of the gates above. No firing path can carry it — a narrow
unsigned IV must present an `unsigned_iv_bound` before any recurrence is armed, and
that function refuses every step except `±1` — but it is the first thing a
descending-loop implementation has to fix, and reading the step through
`const_in_iv_domain` (the helper A4 added for exactly this class of confusion)
turns `+2^32 - 1` into `-1` and makes the descending arm live. That is a feature
with a measurable blast radius, not a hardening, so it is filed under PERF-4 with
the `Ne` polarity work rather than slipped into a review-response round.

### 2.3 And the reason no descending recurrence fired at all

All sixteen probes ended without a recurrence, including the ten that formed an
IV, for a third reason independent of the proof gates: `[IVSR] no derived exprs`.
A backwards byte walk is already SIB-indexed (`movzbl (%rdi,%rsi)` — 5
instructions per iteration, nothing to strength-reduce), and where an offset
instruction does exist it is scaled *after* a widening `Cast`, which
`find_derived_exprs` does not collect (PERF-6, measured at 0 of 402 corpus
shapes). Descending-loop IVSR therefore needs the collector fixed before the
proofs matter — which reorders PERF-4 ahead of PERF-6's `Sub` support.

---

## 3. F7 — the end-to-end test asserted nothing, proven by revert

`descending_unsigned_address_loop_end_to_end` forms its offset by widening first
(`Cast U32 → I64`, then `Add(base, off) : I64`). The address-forming collector
requires `ty.size() == ptr_ring` and records `mul_ty: *ty`, so `mul_ty` is `I64` —
and `offset_product_cannot_overflow` begins with

```rust
if !mul_ty.is_integer() || (mul_ty.size() as usize) >= ptr || !mul_ty.is_unsigned() {
    return true;   // pointer ring: a product cannot wrap the address space
}
```

so `lo`/`hi` are never read. The only thing the bound contributes to that fixture
is `is_none()` at the caller gate, and the pre-fix bound was `Some` too.

**Proof, not argument.** A scratch worktree at the merged head with my file copied
in, then the A1 hunk reverted to the pre-PR-#774 bound (`hi = limit - 1` for both
directions, flag inherited from the limit's constness):

```
test …::descending_unsigned_address_loop_end_to_end                       ... ok      <- asserts nothing
test …::descending_narrow_offset_product_certificate_is_the_operative_proof ... FAILED  <- discriminates
test …::descending_unsigned_iv_bounds_come_from_the_init_not_the_limit      ... FAILED
panicked at src/passes/iv_strength_reduce.rs:3000:9:
a descending offset that wraps its own ring must not be reduced
```

(`worktree remove --force`; the main tree was never dirty. The shared
`CARGO_TARGET_DIR` was rebuilt afterwards, so no reverted artifact survived into
any measurement.)

The old test's comment claimed the pre-fix bound was "inexact", that "the numeric
certificate was withdrawn" and that "the loop was declined outright". All three
were false: `exact` was `true` for that literal limit, nothing was withdrawn, and
`hi = limit - 1 = -1` sits *below* the range — an admission, not a refusal. The
comment now says what the fixture does and does not exercise, and points at the
one that does.

### 3.1 The fixture that carries the certificate

`descending_narrow_gep_loop(init)` scales inside the IV's own ring and hands the
product to a GEP, so `mul_ty == U32` and the numeric check is the operative proof:

```
header: i = phi(init, i_next) : U32 ; cmp Ugt(i, 0) : U32
body:   off = i << 2 : U32 ; p = GEP(base, off) : I64 ; v = *p ; i_next = i + -1 : U32
```

| cell | executed offset range | pre-fix (`hi = limit - 1 = -1`) | post-fix (`hi = init`) |
|---|---|---|---|
| `init = 64` | `[4, 256]` | certified, reduced | certified, reduced |
| `init = 2^30 + 1` | max `(2^30+1)*4 = 2^32 + 4`, **wraps U32** | `-4 < 2^32` passes → **reduced** | `2^32 + 4 >= 2^32` → **declined** |

The second cell is A1 as a miscompile rather than as an argument: pre-fix, a
pointer recurrence bumping by `-4` at 64-bit width was armed for an offset the
source computes modulo 2^32, so the first address would have been
`base + 2^32 + 4` where the source says `base + 4`. That this is unreachable from
C today (§2.1) makes it latent; it does not make it a test that asserts nothing.

---

## 4. F2 — `exact` → `bounded`

The field used to mean "the limit was a compile-time constant", which is the wrong
question for a descending IV: the limit fixes the **floor** there and the init
fixes the **ceiling**. A loop-invariant limit with a literal init therefore
withdrew a certificate the init had already earned.

| arm | `lo` | `hi` | old flag | `bounded` now |
|---|---|---|---|---|
| ascending, literal limit | `0` | `limit - 1` | `true` | `true` (unchanged) |
| ascending, invariant limit | `0` | type max | `false` | `false` (unchanged) |
| descending, literal init, literal limit | `limit + 1` | `init` | `true` | `true` (unchanged) |
| **descending, literal init, invariant limit** | `1` | `init` | **`false`** | **`true` — recovered** |
| descending, literal init below the floor | `floor` | `floor` | `true` | **`false`** — an empty body has no maximum |
| descending, non-literal init | `limit + 1` | type max | `false` | `false` (unchanged) |

`floor = 1` is sound for every descending unsigned IV: entry requires
`init > limit` and `limit >= 0`, so every observed value is `>= 1`.

The struct doc now states the invariant that A1 violated, because a name is not a
proof: `[lo, hi]` must be sound in **both** directions (`lo <= ` the smallest body
value, `hi >= ` the largest). Over-approximation costs an optimization;
under-approximation is a miscompile. `bounded` is about strength, not soundness —
it says `hi` is the exact body maximum, so a product certificate may be issued.

---

## 5. The documentation and ratchet defects

* **F3** — `ENV_READ_BUDGET` was 155 with 153 measured, under a header that says
  to lower it and never raise it. Now 153, with the cause recorded: IVSR reads
  `CCC_IVSR_DEBUG` once per invocation instead of once per trace site
  (`iv_strength_reduce.rs` 5 reads → 3). Verified:
  `check_env_test_hygiene.sh` → `ok pass-pipeline environment reads: 153 (budget 153)`.
* **F4** — `is_used_as_address` documented itself as "conservative by
  construction: anything not positively identified as a memory operand returns
  false", which is the **opposite** of what it does and of what the call-site
  comment and `address_classification_deliberately_over_approximates_intrinsics`
  say. Rewritten to name the delegation to `IntrinsicOp::reads_pointer_arg`, the
  A3 measurement that forbids narrowing it (`simd_crc_adler` +7 insns/+4 stack,
  `simd_vecreg` +2/+2), and the soundness argument (only pointer-width values can
  be armed; C17 6.5.6p8). A contradiction between two comments in the same file is
  a shipped defect, not a nit.
* **F5** — the census comment quoted `grep -RIn '\.result_type()' src` and reported
  18 sites in 10 files; the line quoting the pattern matches it, so the command
  returned 19 in 11. The documented command now filters its own line
  (`… | grep -v CENSUS`) and reproduces the documented numbers exactly — verified
  18 lines / 10 files, per-file itemisation unchanged.
* **F6** — `const_in_iv_domain` claims to be *the* single definition of a
  constant's value, but it reads the carrier with `to_i64()`, which truncates
  `IrConst::I128`. Benign and now scoped in the doc: the bound is only consulted
  for a modular IV (at most 32 bits on every supported target) whose exit test
  compares in `iv.ty`, and `bits >= 128` returns the raw value, so the masking
  branch never sees a 128-bit domain. Documented rather than widened: an
  `i128`-exact reader would be dead code whose failure mode is silently believing
  a truncated constant is a narrow one.
* **F8** — the evidence `README.md` described the **first** A/B run (1232
  comparisons over two corpora) while `static-neutrality.json` records the last
  (1480 over three). Corrected, with the per-corpus split now visible in the row.
* **Not in the audit** — `scripts/ci_local.sh` was mode `100644`, so the invocation
  its own documentation uses in nine places (`./scripts/ci_local.sh --fast`) fails
  with `Permission denied`. It is one of only two non-executable `.sh` files in
  `scripts/` out of 55, i.e. an outlier rather than a convention. Mode set to
  `100755`; the other (`qemu_icount_plugin/build.sh`) is only ever referenced in
  prose and through `bash`, so it was left alone. A mandatory gate that cannot be
  run as documented is a defect in the gate, not in the reader.
* **F9** — `backlog.md` named `address_add_arm_is_gated_by_the_kill_switch_and_ilp32`,
  which the previous round split into `ptr_add_parameter_is_a_real_kill_switch` and
  `address_add_arm_is_gated_by_ilp32`. Corrected to four tests, with the rename
  recorded so the entry does not look wrong against an older revision.

---

## 6. What this round deliberately did not do

* **No descending-loop IVSR.** §2 gives the design (normalise the step through
  `const_in_iv_domain`, accept `Ne`-against-zero for a unit decrement) and the
  reason it is not in this round: it arms recurrences no C program reaches today,
  so its value is unmeasurable until PERF-6's collector gap is closed, and the
  audit's own do-not-touch list puts range/proof logic off limits beyond W2.
* **No loose-bound certification.** Setting `bounded = true` for `hi = type max`
  would be *sound* (the interval still contains every body value) and would newly
  certify stride-1 narrow offsets — byte walks with an invariant limit. That is an
  optimization change in common code, so it needs its own A/B and Callgrind round.
  Recorded here so the omission is a decision rather than an oversight.
* **A3 not re-litigated**, no predicate loosened to make a test pass, the oracle
  re-run kept, and neither rejected experiment (SELECT-CHAIN-FOLD,
  RANGEFOLD-OR-2) revived.

---

## 7. Validation

| gate | command | result |
|---|---|---|
| unit tests | `cargo test --lib` (fastbuild, `-O1`, `-j2`) | **4156 passed, 0 failed, 7 ignored** (merged base 4155: one test added, two renamed) |
| assembly A/B | `diff_corpus.py` + `ab_regression.py`, `PRE=/tmp/lccc-main-774` (unmodified `9d85b134`), `POST=/tmp/lccc-audit2` | **1480 comparisons, 0 changed, 1 skipped**; per-corpus 402 / 844 / 234 with instruction and stack totals byte-identical to the previous round (98002+25745, 232712+39696, 35844+6978) |
| executed regression corpus | `CCC_VALIDATE_SSA=1 run_regression.py --lccc target/fastbuild/lccc -j2` | **887 passed, 0 failed, 13 skipped-compare, 0 skipped-run, 900 total**, exit 0 |
| IVSR domain gate | `check_ivsr_domains.sh` | exit 0 (`gcc-multilib` + `libc6-dev-i386` present, so the `-m32` legs run instead of skipping) |
| four-vendor oracle | `godbolt_oracle.py --oracle-set all-vendors` at `-O2` | **9 agree, 0 diverge, 0 error**; LCCC 1512 vs GCC 16.2 1852 (0.8164), Clang 23.1.0 2274 (0.6649), ICC 2021.10 3714 (0.4071), ICX latest 2441 (0.6194) — identical to the merged base, as byte-identical assembly requires |
| environment ratchet | `check_env_test_hygiene.sh` | `ok pass-pipeline environment reads: 153 (budget 153)` |
| lint / format | `cargo clippy --all-targets`, `cargo fmt --all --check` | no diagnostics / clean |
| local CI | `./scripts/ci_local.sh --fast` on the staged tree | green, exit 0. `--fast` skips only the three slow gates, which are GitHub CI's and are not run on this host by standing instruction; the per-gate count is recorded in the delivery ledger rather than predicted here |

Not re-run, and why: **Callgrind**. Identical assembly bytes on all 1480
comparisons implies identical `Ir`, so the merged round's Callgrind numbers stand;
re-measuring would reproduce them. No wall-clock figure is claimed anywhere in this
round — this host has a ~15% layout-noise floor and no PMU, and it is not the
i7-14700KF target.

Evidence: [`evidence/2026-10-07-ivsr-audit-response/`](evidence/2026-10-07-ivsr-audit-response/README.md).
