# Follow-up: IR structural verification driven to zero, and the first
# measured backend win from the new benchmark harness

Session date: 2026-09-01
Base: `ms178/lccc` main @ `453cbea` (rebased from `2bab8c2`)
Snapshots this session: `S06`–`S10` (see `artifacts/SNAPSHOT_LEDGER.md`)

---

## 1. Accomplished

### 1.1 Corpus IR structural violations: 396 configs → 0

The previous session added `src/passes/verify.rs` (an inter-pass IR structural
verifier behind `CCC_VERIFY_IR`) and found that **396 of 1112** test-corpus
configurations produced malformed IR somewhere in the pass pipeline. This
session drove that to **zero**, and to zero across **all six** optimisation
levels (3336 configurations).

| Defect | Pass | Configs cleared |
|---|---|---|
| IV phi left unresolved after complete unroll | `loop_unroll` (two-block path) | 340 |
| Phi rewritten to `Copy` in place, stranding later phis | `univsr` | 8 |
| Same, in trivial-phi collapse | `cfg_simplify` | 2 |
| Unreachable-predecessor false positives | *(verifier refinement)* | 44 |
| Dead-block stale predecessor false positives | *(verifier refinement)* | 4 (SCCP) |

#### (a) `loop_unroll::try_complete_unroll_two_block`

After a complete unroll the back-edge is gone, so the header has exactly one
predecessor and executes once. The pass rewrote only the phis in `final_map`
(the loop-carried accumulators, which outside code reads *after* the loop) into
`Copy` of the final clone's value, and left the induction-variable phi
untouched. That phi was then malformed twice over: it still named the latch as
a predecessor (`STALE_PRED`), and because the carried phis ahead of it had
become `Copy`s it now sat after a non-phi (`PHI_ORDER`).

The sibling path `try_complete_unroll_general` (`:1029-1045`) had always done
this correctly, with a comment explaining why `incoming.first()` is the wrong
choice; the two paths had simply diverged.

Fix: a single pass over **the header's instructions only** resolving *every*
remaining phi — in `final_map` → `Copy` of the final clone; otherwise → `Copy`
of the non-latch incoming. This is also a compile-time win: `final_map` is
built exclusively from `func.blocks[header]`, so the previous
`final_map` × every-block × every-instruction sweep could never match anything
outside the header.

Latent, not benign: DCE removes the dead IV phi before codegen, but every pass
scheduled between `loop_unroll` and DCE (`narrow`, `simplify`, `bit_idioms`,
`reassoc_accum`, `sccp`, `constfold`, `if_convert`, …) consumed invalid IR.
This is the same shape as the `loop_rotate` defect that produced a real SIGSEGV
once SCCP became the first consumer that trusted the predecessor list.

Reproducer: `tests/regression/unroll_header_phi_resolve.c`. Verified as a true
negative control — with the fix reverted it reports both `PHI_ORDER` and
`STALE_PRED` attributed to `loop_unroll`; with the fix, clean.

#### (b) `univsr` and `cfg_simplify`: in-place phi → `Copy`

Both rewrite a phi into a `Copy` **in place**. When the rewritten phi is not
the last one in the block, every phi after it is suddenly preceded by a
non-phi. A loop header commonly carries several pointer IVs, so reverting one
of them left the rest malformed — the common case, not a corner case.

Fix: new shared helper `passes::restore_phi_prefix(&mut BasicBlock)`. Sinking
the non-phis below the phi prefix is *always* sound: phis are defined to
execute simultaneously on entry to the block, so no phi may read a value
defined by a non-phi in that same block (a back-edge operand is evaluated at
the end of the predecessor, not here). Stable partition, O(n), allocates only
when a repair is needed, and permutes `source_spans` identically — or leaves
them alone when they are already desynchronised from `instructions`, which
passes have been observed to do. Seven unit tests.

#### (c) Verifier refinement: reachability

44 `MISSING_PRED` and 4 `STALE_PRED` reports were false positives.

First, the question left open last session — *does the dialect permit an
implicitly-undef incoming?* — is now settled: **no**.
`phi_eliminate::emit_single_phi_copies` (`phi_eliminate.rs:363`) iterates only
over the phi's *incoming* list, so a real predecessor absent from that list
gets no copy and the register holds whatever was there before. It is a genuine
undefined-value defect.

But that reasoning only bites when the edge can execute. The three checks that
concern a phi's *edge set* (stale / duplicate / missing predecessors) are
vacuous for an unreachable edge or an unreachable block, and passes
legitimately exploit that: SCCP folds a constant `Switch` to a single `Branch`
and deliberately leaves the orphaned blocks alone, documenting that
`cfg_simplify` will delete them. Flagging that punishes correct behaviour.

Those three checks are now gated on reachability. Phi **contiguity** is
deliberately *not* gated: passes index the phi prefix arithmetically
(`loop_rotate` scans instructions at block start) without checking
reachability, so that invariant must hold everywhere. Four unit tests pin both
halves of the rule.

#### (d) Attribution blind spot

`if_convert` was blamed for 8 configurations it did not cause. Phases 5–6
(`run_gvn_licm_ivsr_shared`) and `load_forward` are called directly rather than
through `timed_pass!`, so they were never verified and their defects surfaced
at the next *wrapped* pass. New `verify::verify_after_func_pass(func, stage)`
hooks now cover `gvn`, `licm`, `ivsr`, `univsr` and `load_forward`; the culprit
immediately re-attributed itself from `if_convert` to `univsr`.

**Generalisable lesson**: a verifier is only as good as its hook density.
Any pass not wrapped in `timed_pass!` silently frames the next one that is.

### 1.2 The verifier is now a permanent gate

`scripts/run_regression_suite.sh` compiles every test with `CCC_VERIFY_IR=1`
and fails the test if any `[ir-verify]` line appears (`LCCC_IR_VERIFY=0` opts
out). All 556 tests are now structural-IR tests in addition to output tests.

Cost: none measurable (90 s vs ~95 s before). Proven to fire: with the
`cfg_simplify` fix reverted the suite reports
`FAIL bool_thread_fse_tail_overrun (IR verifier: malformed IR between passes)`.

This was the standing long-term goal recorded last session. It is done.

### 1.3 New tool: `scripts/ir_verify_sweep.py`

Sweeps the corpus, classifies violations by kind, and attributes each to the
**first** reporting pass — necessary because one defect is re-reported by every
subsequent pass, so the noisiest pass is usually just the most frequent one,
not the culprit. Supports `--baseline`/`--save` for per-bucket regression
diffing, printing `IMPROVED`/`REGRESSED`.

Corollary worth remembering: fixing the first reporter can make a later pass's
count *rise*. That is **unmasking**, not regression — it happened here when
`loop_unroll` was fixed and `if_convert`'s bucket went 6 → 8.

Whole corpus in ~32 s (2 levels) / ~100 s (6 levels).

### 1.4 Oracles brought current — Clang 23.1

`scripts/godbolt.py` pinned `clang` to `cclang2210` (22.1.0) while **23.1.0 had
shipped**, so every "we match Clang" claim was measured against a superseded
compiler. Now:

- `clang` → `cclang2310` (23.1.0); `clang22.1` retained for LLVM 22→23 bisects
- `gcc-trunk` → `cgsnapshot` added alongside `gcc16.2` → `cg162`
- verified against the live API that **GCC 16.2 and ICC 2021.10.0 are already
  the newest available** (ICC Classic ended there); ICX stays on the moving
  `cicxlatest` channel and is recorded per-run in every manifest
- new **`godbolt.py audit`** subcommand re-derives the newest release of each
  family from the API and exits non-zero if a pin has gone stale or an alias no
  longer resolves. The oracle set can no longer rot silently.

```
ALIAS     PINNED ID       PINNED      NEWEST      STATUS
gcc       cg162           16.2        16.2        ok
clang     cclang2310      23.1.0      23.1.0      ok
icc       cicc2021100     2021.10.0   2021.10.0   ok
```

### 1.5 New runtime benchmark harness + corpus

Instruction counts from the Compiler Explorer oracle say which compiler emits
*fewer* instructions, not which emits *faster* code. `scripts/bench_kernels.py`
measures wall-clock time of the same kernel compiled by each compiler.

Method (no PMU in this VM): one translation unit per kernel per compiler,
linked against a driver compiled once by a fixed reference compiler so only the
kernel's codegen differs; best-of-N samples (minimum, not mean — least
perturbed, one-sided distribution); arms interleaved round-robin so drift hits
each equally; a checksum through a volatile sink means an arm that computes
something different is reported as a **CORRECTNESS failure, not a speed win**.
`--baseline`/`--save` gate runtime regressions.

`tests/bench/` corpus, all drawn from the target workloads:
`k_adler32` (zlib-ng), `k_matchlen` (gzip/zlib `longest_match`), `k_namechars`
(expat), `k_varint` (SQLite), `k_memchr` (glibc), `k_hashmix` (FNV-1a).

**Baseline established — lccc is currently ~1.6x slower than GCC 14.2 geomean.**
This is the most useful number produced this session: it is a concrete, honest,
reproducible scoreboard where none existed.

### 1.6 First measured backend win: −35.3% on the gzip compare loop

The `matchlen` inner loop showed lccc emitting:

```
.LBB8:
    movslq %r15d, %r15          # sext the index
    movzbl (%r12, %r15), %r10d
    movslq %r15d, %r15          # SAME src, SAME dst, nothing redefined it
    movzbl (%r13, %r15), %eax
    cmpl %eax, %r10d
```

The IR was **optimal** — one `Cast->v43`, two GEPs sharing it as their offset.
Address lowering re-materialised the extension per GEP. This fires on any
shared-index expression (`a[i]` and `b[i]`), which is pervasive.

Fix in `redundant_ext.rs`: a fourth tracked fact, `sext32[f]` = "family f's
64-bit value is provably `sext(its own low 32 bits)`" — exactly the
postcondition of `movslq %fd, %f`. A second such instruction is then a no-op.
Established **only** by `movslq`, propagated **only** by register-to-register
`movq`, cleared by every other write, by calls, and at every label. A
zero-extending write must *not* set it: `movl` leaves bits 32..63 clear, but if
bit 31 is set then `sext(low32)` sets them all instead.

| | before | after | |
|---|---|---|---|
| inner loop | 10 insns | 9 insns | |
| `matchlen` runtime | 607.5 ms | **393.3 ms** | **−35.3%** |
| geomean vs GCC | 0.579x | 0.620x | |

35% from one instruction in ten is superlinear, and the reason matters: the
extension sat on `%r15`, the **loop-carried** register, so it lengthened the
recurrence critical path rather than merely adding issue slots. Removing
dependencies on the carried register is worth far more than the instruction
count suggests — a heuristic worth applying to the remaining gaps.

### 1.7 A pre-existing soundness bug found by writing negative tests

Two of the six negative controls written for the above **failed**, exposing a
latent defect that predates this session:

```rust
if t.is_empty() || t.starts_with('.') { continue; }   // directives
if t.ends_with(':') { /* clear all register facts */ }  // labels
```

The directive skip ran **first**, so `.LBB1:` — the form lccc emits for *every*
basic-block label — was skipped as a directive and never cleared the facts.
Zero-extension facts therefore leaked across basic-block boundaries: a block
entered from several predecessors inherited whatever the textually preceding
block happened to leave behind, and a re-extension genuinely needed on one
incoming path could be deleted. Only `foo:`-style labels were handled.

Fixed by testing for the label form first (exact, not merely conservative:
real directives do not end in `:`). Full suite green afterwards.

**Lesson: the negative controls found a bug the positive test could not.**
Six of the eight new peephole tests assert that a transformation does *not*
happen. That ratio should be the norm for peephole work.

### 1.8 Honest negative result: loop rotation is not the bottleneck

`loop_rotate` is opt-in behind `CCC_LOOP_ROTATE=1`, and since last session's
fix made it correct, enabling it by default looked attractive. Measured A/B
across all six kernels: **1.00x** on five and **0.87x (slower)** on `adler32`.
Not enabled. Recorded here so no future agent re-runs this experiment.

---

## 1.9 Rebase onto `453cbea`, and a defect in *our own* fix

Rebasing the four session commits onto upstream `453cbea` (6 new commits:
popcount SWAR, RA-09 commutative-ALU fix, PF-17 loop-rotation hardening)
produced exactly one conflict, in `try_complete_unroll_two_block` — because
**upstream's PF-17 fixed the same defect independently**, and their resolution
exposed a real bug in ours.

Both versions resolve *every* header phi, which is the structural fix. They
differ in the value used for the loop-carried ones:

| | carried phi resolves to |
|---|---|
| ours (S06) | the **final clone's** value |
| upstream PF-17 | the **non-latch (init)** incoming, like every other phi |

Upstream is right, for two independent reasons:

1. **Copy-before-def.** The clone that defines the final value runs *after* the
   header, so the definition does not dominate the use. GVN/copy-prop then
   forwards an unordered value into inlined callers — upstream observed
   `simd_crc_adler` returning `00010001` instead of `00bd00bc` for `sz=2`.
2. **It is unnecessary.** Outside uses of a carried phi were *already* rewritten
   to the final clone's dest by the substitution loop that runs immediately
   before, which visits every block not in `lp.body`. Nothing outside still
   reads the phi id.

This was verified rather than assumed. Both variants pass the entire corpus —
559/559, rotation on *and* off — because the emitted programs still print the
right answers; only the IR is malformed. Dumping the IR after `loop_unroll` on
our own reproducer settles it:

```
[OLD semantics (final clone value)]  2 copy-before-def
    block 3 (.LBB4): Copy v50 = v146  <-- v146 defined later, in block 22
    block 3 (.LBB4): Copy v51 = v147  <-- v147 defined later, in block 22
[RESOLVED (upstream init-value semantics)]  0 copy-before-def
```

Resolution shipped: upstream's semantics, plus two things carried over from
ours that upstream does not have —

- a `debug_assert!` when a header phi has **no** non-latch incoming. Upstream
  silently leaves such a phi in place, which would preserve the exact
  structural violation the loop exists to remove. Impossible for a natural
  loop, so making it loud costs nothing in release builds.
- a `debug_assert!` that the chosen operand is **not** one of the clone values,
  so a future edit cannot reintroduce the copy-before-def. The corpus provably
  cannot catch that regression, so the guard has to live in the code.

The comment block records both failure modes side by side, and
`tests/regression/unroll_header_phi_resolve.c` — whose header comment
previously asserted the *wrong* behaviour and has been corrected — now states
explicitly that the output check cannot catch this and why.

**Lesson, and it is the sharpest one of the session:** our fix was validated by
1392 unit tests, 556 regression tests, an A/B differential, and a
structural-IR sweep over 3336 configurations — and it still contained an SSA
violation, because *none of those gates checked def-dominates-use*. A green
board measures the checks you wrote, not correctness. §3.4's dominance check is
consequently promoted to the top of the verifier backlog.

### Bonus finding: the corpus is now green with rotation ON

PF-17 explicitly kept `loop_rotate` opt-in "until the full 474-test corpus is
green". With PF-17's guards D/E combined with this session's IR fixes, the
current tree is:

| `CCC_LOOP_ROTATE=1` | result |
|---|---|
| regression suite | **PASS=559 FAIL=0 SKIP=15**, AB-diff 0 |
| IR verifier sweep | **0 violations** / 1118 configs |

That is the stated precondition for flipping the default, now met. It is *not*
flipped here: §1.8 measured rotation at 1.00x on five kernels and 0.87x
(slower) on `adler32`, so the case for default-ON has to be made on wider
performance evidence, not on correctness alone. Recorded for whoever takes
that decision.

---

## 2. Validation

| Gate | Result |
|---|---|
| `cargo test --lib` | **1393 pass / 0 fail / 6 ignored** (was 1372) |
| `./scripts/run_regression_suite.sh` | **PASS=559 FAIL=0 SKIP=15**, AB-diff 0 |
| same, `CCC_LOOP_ROTATE=1` | **PASS=559 FAIL=0 SKIP=15**, AB-diff 0 |
| `./scripts/ir_verify_sweep.py` | **0 violations** / 1118 configs |
| `--levels O0 O1 O2 O3 Os Oz` | **0 violations** / 3354 configs |
| `bench_kernels.py --baseline` | no runtime regressions; `matchlen` −35.3% |
| `godbolt.py audit` | all pinned aliases current |

---

## 3. To do next, in priority order

### 3.1 Close the remaining runtime gap vs GCC (highest value)

Current standing, best-of-7, `-O3`, lccc vs GCC 14.2 (>1 = lccc faster):

| kernel | vs gcc | headline gap |
|---|---|---|
| `memchr` | **0.47x** | worst; investigate first |
| `namechars` | 0.48x | byte classification, if-conversion |
| `varint` | 0.62x | branch layout on a serial chain |
| `matchlen` | 0.66x | two gaps left, both identified below |
| `adler32` | 0.78x | reduction/unroll quality |
| `hashmix` | 0.81x | closest; multiply chain scheduling |

Two concrete, already-diagnosed defects remain in `matchlen`, both visible in
the assembly quoted in §1.6:

1. **No load-compare fusion.** lccc emits
   `movzbl (%r13,%r15), %eax; cmpl %eax, %r10d`; GCC emits the single
   `cmpb %dl, (%rdi,%rax)`. One instruction and one register saved per
   iteration, in the hottest loop shape in gzip/zlib.
2. **Loop not rotated in the emitted code.** lccc has `cmpl/jge` at the top and
   an unconditional `jmp` back — two branches per iteration where GCC has one.
   Note §1.8: the IR-level `loop_rotate` pass does *not* fix this, so the gap
   is in block layout/branch emission, not in the IR pass. Investigate the
   backend block ordering.

Method that worked and should be repeated: `bench_kernels.py` to rank the gaps,
`-S` diff against GCC on the single worst kernel, find the specific instruction
pattern, fix it, re-measure with `--baseline`.

### 3.2 Extend the benchmark corpus

Six kernels is enough to rank gaps but not to prevent regressions broadly. Add
excerpts from the remaining target workloads (SQLite b-tree compare, Linux
kernel `memcpy`/checksum, zlib-ng CRC32, expat tokenizer) sourced from
`https://github.com/ms178/archpkgbuilds/tree/main/packages`, and wire
`bench_kernels.py --baseline` into the standard validation sequence so a
codegen change that costs runtime is caught the same way an IR defect is.

Also add `clang` as a third arm once a local Clang is available in the image
(only GCC is installed today, so the `vs clang` column is empty).

### 3.3 Audit the other peephole passes for the same label bug

§1.7's defect was in `redundant_ext.rs`, but the
`if starts_with('.') { continue }` idiom may well be duplicated across the
other ~20 passes in `src/backend/x86/codegen/peephole/passes/` and the
arm/i686/riscv peepholes. Any pass that carries per-register facts across lines
and skips dot-labels has the same cross-block leak. This is a **correctness**
audit and should precede further peephole optimisation.

### 3.4 Keep the structural gate honest

- **Highest-priority verifier work: def-dominates-use.** The verifier checks
  six structural properties, none of which is dominance — which is why our own
  `loop_unroll` resolution shipped an SSA violation past every gate (§1.9). It
  would also have caught the `loop_rotate` defect *directly* rather than via
  the predecessor list. `verify.rs` already computes reachability; adding
  Cooper-Harvey-Kennedy dominators over RPO is the natural next step. Expect to
  triage a backlog on first run, exactly as with the six existing checks — run
  `ir_verify_sweep.py` to size it before committing to fixes.
- Consider running the gate at `CCC_VERIFY_IR=abort` in CI so a violation
  cannot be merged even if someone ignores the suite output.

### 3.5 Standing items not advanced this session

- Linker oracle: honour the user's build/version preferences (lld 23.1,
  mold 2.42 with the X86/i686-only preset, bfd 2.47).
- `lccc-bootstrap.sh` still lacks the rustup 1.98.0 reinstall step; every
  harness wipe currently needs it done by hand.
- Remaining `/scripts` analysis/improvement pass.

---

## 4. Notes for whoever picks this up

- **Attribute to the first reporter.** Raw `CCC_VERIFY_IR=1` output is
  misleading read directly; one defect is re-reported by every later pass. Use
  `ir_verify_sweep.py`, and remember a later bucket rising is unmasking.
- **A verifier only sees what it is hooked into.** If a pass is not wrapped in
  `timed_pass!`, add a `verify_after_func_pass` call before trusting blame.
- **Write the negative control before believing the positive test.** Both the
  `loop_unroll` reproducer and the peephole rules were only trusted after the
  fix was reverted and the test was confirmed to fail. One earlier structural
  test passed with the fix reverted and was therefore worthless as a guard; a
  second synthetic loop never even reached the code path it claimed to pin
  (`find_iv_in_loop` rejected its shape) and was deleted rather than kept as
  false comfort.
- **Do not re-add** a test asserting that no phi survives a loop header after
  `unroll_loops`: partial unrolls legitimately keep a back-edge phi.
- `/tmp` is wiped between tool calls; heredocs fail in the bash tool (use
  `write_file` or `python3 - <<'PY'`).
