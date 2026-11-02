# Follow-up 2026-10-09: iv_widen theorem 5′ — exposed-latch trajectories, label resolution, latch twins

Status: implemented, validated, committed on the unified research branch
(base `500e652e`, main). All numbers in this document were produced on the
committed tree by the recorded commands; nothing is quoted from memory.

## 1. What was wrong

Three defects in the unsigned counted-loop widening (`src/passes/iv_widen.rs`,
LP64, unified tree gates the pass at `-O2`+; main gates it from `-O1`):

### 1.1 IVW-RANGE-1 — live miscompile (severity: silent wrong answers)

Rotated/bottom-tested loops (`do { … } while (i++ < n)`) evaluate the latch
step `phi ± mag` on the *exiting* pass, at a value the guard never tested.
With a runtime seed at the 32-bit ring top (`i = 0xFFFFFFFF`), the narrow
recurrence wraps to `0` where the wide (U64) recurrence reaches `2^32`. The
old pass proved a range for the *tested* values only, widened the loop, and
let loop-exit consumers read the divergent wide latch value directly:

```c
uint32_t i = v_seed;               /* 0xFFFFFFFF, volatile-routed */
do { acc = buf[i & 63u]; } while (i++ < n);
return acc + buf[i];               /* narrow: buf[0].  old lccc -O2: buf[2^32]. */
```

Executed evidence (probe `tests/regression/ivwiden_exposed_latch.c`, markers
past the ring top in a `2^32+3`-byte mmap span):

| binary | A | B | C | D |
|---|---|---|---|---|
| gcc -O2 (oracle) | 11 | 11 | 21 | 44 |
| pre-fix lccc -O2 | **99** | 11 | **91** | 44 |
| fixed lccc -O0..-O3 | 11 | 11 | 21 | 44 |

Case C is the twin-fold sibling: a body `buf[i + 2u]` (a second `Add(phi,2)`
the frontend emits because iv_widen runs ahead of the merging CSE) was
admitted by a fold from the strict range, and the wide twin addressed
`2^32+2`.

### 1.2 Obligation P compared labels against indices

The exit-edge obligation resolved branch *targets* (`BlockId` labels) against
the loop body (block *indices*). After any CFG simplification the numbering
domains diverge — every real frontend dump in this session had gaps (body
index 3 under label 5; probe labels 11/13 for indices 1/2; 17/19 for 2/3) —
and every well-formed loop silently failed P. The whole ring-idiom family
lost its widening with no diagnostic. Fixed by resolving labels through a
`label_to_idx` map before any body membership test.

### 1.3 The latch twin (over-refusal)

`a[(i + 1) & m]` with `while (i++ < n)` compiles to two value-identical
`Add(phi,1)` instructions in the latch block. The twin (the body's copy)
failed the seed-inclusive fold at the ring top and declined an otherwise
sound plan. `canonicalise_latch_twin` now redirects the back edge to the
earlier twin and deletes the dead original (guarded: only when the latch
value has no uses beyond the phi incoming — the liveness guard is pinned by
`test_twin_with_other_uses_is_not_canonicalised`), routing the shape through
the intercept's bit-exact And arm.

## 2. Theorem 5′ (what the pass now proves)

For an unsigned IV phi with latch step `phi ± mag` (mag a positive constant
or, for runtime steps, exactly 1) the widening is sound iff:

* **Trajectory gates.** The body is entered only through the header
  (`body_entry_closed`: every body block except the header has all
  predecessors inside the body); the latch's only in-body successor is the
  header (`latch_on_inner_cycle` declines otherwise — a latch on an inner
  cycle steps without re-passing the exit test and outruns any bound); the
  exit obligation P resolves labels→indices and requires exactly one arm to
  stay in the body (both-in or both-out declines); signed predicates on
  unsigned phis decline outright (a signed test confines an unsigned value
  to no interval: `0xFFFFFFFF <s 64` is true).
* **Polarity normalisation.** Exit-on-true arms are negated (`i > t` exits
  ⇔ `i ≤ t` continues ⇔ upper bound `t+1`); non-strict forms shift the
  constant threshold only (runtime thresholds accept strict forms only).
* **Position split.** `cmp` inside a *clean* header (phi + cmp + terminator
  only) gates every body pass ⇒ **strict** range `[0, thr−1]` (upper) or
  `[thr+1, max]` (lower) — the seed itself is never tested but also never
  *used* before the first test. Anything else (cmp in the latch block, or a
  dirty header) ⇒ the seed's body pass is untested ⇒ **seed-inclusive**
  fold: const seed `c` extends the strict range with `c` (upper:
  `[0, max(c, thr−1+mag)]`; lower: `[min(c, thr+1−mag), max]`), runtime
  seed ⇒ the full type range.
* **Bound leaves room** for the exiting latch step in *every* direction and
  magnitude (`thr ≤ max − mag` upper; `thr ≥ mag − 1` lower): a `Ule`-style
  threshold at `max+1` wraps the latch itself.
* **Exposed-latch intercept.** When the latch is exposed (cmp not gating the
  seed pass) *and* the proven range does not close the exiting step
  (`hi + mag > max`, or `lo − mag < 0`), member values can diverge from
  `zext(narrow)`. The plan is still admissible, but every use is classified
  by *value exactness*: bit-exact `& const` reads stay admissible (they mask
  divergence away for every input — the ring idiom itself), folds inherit
  exactness only when the input was exact, `Or/Xor`/shifts/selects/branch
  conditions on divergent members escape-or-bail, casts and GEP/intrinsic
  offset slots on divergent members **decline the plan** (this is the
  IVW-RANGE-1 repair), out-of-loop escapes are repaired by truncation (the
  low bits of the wide value *are* the narrow value), and the guard cmp of a
  divergent member is forced to the Trunc action so the *narrow* trajectory —
  and with it the counted proof — survives the widening. A wrap-closed range
  (`hi + mag ≤ max` / `lo − mag ≥ 0`) proves no-wrap over the whole
  trajectory and switches the intercept off; this subsumes the old
  exposed-const-seed declines (seed `max` with any positive step is never
  wrap-closed ⇒ intercept on ⇒ the cast/GEP declines fire exactly where the
  old special-case decline fired, and const-seed+runtime-bound now widens
  through bit-exact body reads instead of declining).

Design note (rejected alternative): a general "divergent-member" value
semantics (tracking per-member divergence sets through the closure) was
prototyped on paper and rejected — the twin canonicalisation plus the
exactness-classified intercept covers every executed shape with ~30 lines of
canonicalisation instead of a second range domain, and cannot silently
over-admit.

## 3. Verification battery (all on the committed tree)

| check | command | result |
|---|---|---|
| unit fixtures | `cargo test --profile fastbuild --lib iv_widen` | **50/50** (38 round-4 fixtures — one replaced — plus 12 theorem-5′ fixtures) |
| full lib suite | `cargo test --profile fastbuild --lib` | **4225 passed, 0 failed, 7 ignored** |
| executed regression | `bash tests/regression/check_ivwiden_exposed_latch.sh` (CCC=fixed) | **PASS**, exit 0; 4 opt levels × {default, kill switch, -m32 SKIP} |
| discrimination | same script, `CCC=` pre-fix binary | **FAIL exit 1** at `default -O2` (`A=99 … C=91` in the FAIL line) |
| wrap-ring sweep | `scripts/wrap_ring_sweep.py` const + `--opaque`, `-O0..-O3` | **468/468 agree** (39 const + 78 opaque × 4 opts, gcc reference) |
| regression corpus | `tests/regression/run_regression.py --lccc <fixed> -j 2` | **889 passed, 0 failed**, 13 skipped-compare, 171 s |
| census A/B | `scripts/census_ab.py <final> <fixed2> --opt=-O2 --no-gate` | **189/190 functions identical**; sole delta below |
| callgrind A/B | `scripts/callgrind_ab.py <bin> gcc "-O2" histogram` | Ir **bit-identical** final vs fixed2 (ref 3086096 / mine 3459398, ratio 1.12096; I1/D1/LL/Bcm/Bim columns identical) |
| widen profile | `CCC_IV_WIDEN_DEBUG=1` on the probe | A DECLINE (intercept cast-bail), B WIDEN (twin + And-63 + forced Trunc cmp), C DECLINE (seed-fold overflow), D WIDEN (hoisted bound), main setup-k WIDEN |
| fmt / clippy | `cargo fmt --check`; `cargo clippy --profile fastbuild --all-targets` | clean / exit 0 |
| CI gate parity | `python3 scripts/check_ci_gate_parity.py` | PASS (148 commands) |
| local CI | `bash scripts/ci_local.sh --fast` | see §5 |

### k21_fir4 census delta — documented waiver (cold-only)

`k21_fir4.c`: +4 insns, +2 push, +2 pop, stkref 0, rrmov 0. Re-proven on
the committed binaries: opcode census `{pushq 4→6, popq 4→6}`; total
instruction stream 30→34; the opcode census of everything from the first
loop label onward is **identical** — the delta is one extra callee-saved
register in the prologue/epilogue because a live wide `Shl(V,2)` (I64) now
folds into scaled-index addressing in the inner loop, raising register
pressure by one at function scope. Zero steady-state cost; the census is a
screener, not the gate (its HOT-insns tiebreak trips on any +1 even with
stkref unchanged).

## 4. Files

* `src/passes/iv_widen.rs` — theorem-5′ core: trajectory gates, polarity
  normalisation, position split, room obligation, `CountedProof`
  (`range` + `latch_exposed`), the exactness-classified intercept in
  `analyze_iv`, unconditional unsigned And ranges,
  `canonicalise_latch_twin`, label→index resolution (obligation P), and the
  50-fixture battery (the shared `rotated_loop` builder gives every new
  fixture divergent labels so P stays pinned).
* `tests/regression/ivwiden_exposed_latch.c` — executed 4-case probe
  (self-checking exit codes, ILP32 SKIP path).
* `tests/regression/check_ivwiden_exposed_latch.sh` — gcc-parity matrix
  runner (honours `$CCC`).
* `scripts/ci_local.sh` — `ivwiden-exposed-latch` fast gate.
* `.github/workflows/ci.yml` — hosted mirror step (gate parity).
* `src/backend/generation.rs` — removed a blank line after an outer
  attribute (clippy `empty_line_after_outer_attr` under `-D warnings`;
  invisible to rustfmt and plain rustc).

## 5. CI result

`bash scripts/ci_local.sh --fast` on the committed tree (log
`ci-fast-round5c.log` in the session workspace): **172 passed, 0 failed,
5 skipped, exit 0**, pass stamp on tree `66d81d8d…`. An earlier run of the
same tree had exactly two failures, both environmental and both fixed
before the green run: a check script whose `mktemp` template needs a
`tmp/` subdirectory under a relocated `TMPDIR`, and the linker suite's
`i386_dso_emit_semantics` needing 32-bit C++ headers (`g++-multilib`,
the known S10-era multilib gap on a rebuilt VM). Zero code failures in
any run. The full/slow CI is deliberately NOT run locally (resource
policy: it is the hosted pipeline's job).

## 6. Known limitations / backlog (unchanged priorities)

* PERF-10 collector gate (rotated do-while), PERF-8 range analysis for the
  glibc_strstr class (1.04097 Ir), PERF-9 select-from-comparison (held
  back: breaks `check_select_from_comparison.sh` contract 2 until the RA
  fix lands), PERF-7 ILP32 RA (+60 on the i686 matrix), PERF-5 redundant
  ext/copy (~2 % Ir on histogram), PERF-6 `(I64)(i<<2)` IVSR gap, PERF-4
  descending loops.
* Inner-cycle `continue`-relaxation (a latch successor that re-enters the
  body at a *tested* block could still widen) — only with a live repro;
  the gate currently declines the shape.
* The twin canonicalisation stays even if a future CSE reorder makes the
  duplicate rare: it is 30 lines, liveness-guarded, and fixture-pinned in
  both directions (widen + no-redirect).
* Runtime-magnitude steps (`i += k` with k opaque) prove only for mag 1
  (`i++`/`i--`); larger runtime strides decline (no room obligation
  possible).
