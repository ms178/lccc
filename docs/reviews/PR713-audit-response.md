# PR #713 — audit adjudication and follow-up

**Scope.** The PR #713 review asked for a critical, evidence-led adjudication of
its own findings, fixes for everything that survives scrutiny, and a regression
for every fix. This document is that adjudication: finding by finding, with the
verdict, the evidence, what changed, and how the change is pinned so it cannot
silently rot.

**Method, and a warning about it.** The review was produced by a model with no
build, no test run, and no compiler in the loop. Its findings are therefore
*claims about code*, and a claim about code is worth exactly what a probe says
it is worth. Every finding below was re-derived from the source and then either
reproduced with the compiler or shown to be unreachable — and in one case the
probe showed the finding was right about the mechanism but wrong about the
severity. Two findings were rejected as stated and re-framed; one was rejected
outright; one was accepted and then turned out to be only half the bug.

Where the review's wording is quoted it is from its own text. The review's
findings are labelled here as it labelled them (`P1-a`, `P1-b`, `P1-c`, `P2`)
plus the unlabelled ones by subject.

**Verdict summary.**

| # | Finding | Verdict | Reachable miscompile? |
|---|---------|---------|----------------------|
| 1 | Inclusive-bound miscompile in the packed matmul arm | ACCEPT | **Yes** — 240/289 cells wrong at N=17 |
| 2 | Matcher has no shape proof before the packed rewrite | ACCEPT | Yes, jointly with 1 |
| 3 | FMA transform ignores `-ffp-contract` and the target FMA bit | ACCEPT | Yes — illegal `vfmadd` under `-mno-fma` |
| 4 | `P1-a` — `fold_shift_into_sib` misses `reg_refs` reads | ACCEPT (severity REFRAMED) | No reproducer at C level — latent |
| 5 | `P1-b` — widened-mask tracking truncated to a byte | ACCEPT | Latent, same class |
| 6 | `P1-c` — flags horizon not fail-closed | ACCEPT | Latent (first fix over-refused; sharpened) |
| 7 | `P2` — affine exit-compare algebra inverts the operator | ACCEPT | Yes, in the clone path |
| 8 | Kill switch `CCC_NO_AFFINE_EXIT_FOLD` misses the clone fold | ACCEPT | Yes — coverage hole, not a miscompile |
| 9 | Fused volatile read-modify-write (`h += 3`) violates `volatile` | **REJECT as stated** | No — access count and order preserved |
| 10 | Wrap probes referenced by no gate; dead conditional in `wrap_loop_fold.c` | ACCEPT | Test-coverage defect |
| 11 | *(self-found while writing 9's regression)* A gate whose failure path could not fail | ACCEPT | Gate defect |

Two findings that the review did **not** file and the probes found are in §12.

---

## 1, 2. The inclusive-bound miscompile and the missing shape proof — ACCEPT

**Claim.** The packed matmul arm rewrites `C[i][j] += A[i][k] * B[k][j]` for
loops whose `j` bound is written `j <= N-1`, and the rewrite is applied without
proving the loop's shape.

**Verdict: accept, and it is the most serious finding of the round.** It is not
a theoretical hazard: it produces wrong numbers on the *default* command line.

The probe a reviewer without a compiler could not run:

```
N=17   j<=17-1    lccc=10222.297959    gcc -O0=4196.600000    240/289 cells wrong
N=31   j<=31-1    wrong
N=33   j<=33-1    wrong
N=49   j<=49-1    wrong
N=65   j<=65-1    wrong
```

Two independent defects had to line up, which is why it survived: an in-place
byte-limit rewrite changed the comparison's *operator* while normalising
`j <= N-1` to a length, and the scalar remainder path hard-coded `Slt`
regardless of the normalised relation. Fixing only the first would have left
the second; that is the reason the fix is verified over an envelope rather than
at one point.

**Fix.** `normalize_iv_exit_comparison` normalises the exit comparison without
mirroring the operator (proof: `N op (iv+C) ≡ (N−C) op iv`, operator
unchanged), and the remainder path carries the normalised relation instead of a
constant. The matcher additionally proves the shape before any packed rewrite:
the header condition must be the exit compare's own operand, the exit must be
the sole exit, and the body must contain no call, atomic or volatile access.

**Pin.** The envelope (N ∈ {16,17,31,32,33,48,49,64,65} × {`<`, `<=`}, plus a
*runtime* bound `j <= lim`) is now a gate contract —
`tests/regression/check_fma_gating.sh` contract 5 keeps the runtime inclusive
bound in the fast battery, evaluated against `gcc -O0`, which cannot contract,
reorder or vectorize. The refusal half (call in body, side exit) is contracts
1–4 there. The equivalence oracle for the `P2` algebra is a separate >400-case
module test.

**Note on the flag defaults.** The gating in 3 does **not** change default
output: `-O2` with no `-ffp-contract` is `fast` in the C dialect the reference
compilers implement, so the four packed `vfmadd` remain. The gate removes
output only where contracting was *illegal* (`off`) or *unsupported*
(`-mno-fma`). No performance was traded for correctness here; the same
instruction sequence is now emitted only when it is allowed.

## 3. FMA contract and target gating — ACCEPT

**Claim.** The transform contracts multiplies into FMAs unconditionally,
ignoring `-ffp-contract` and the target's FMA capability.

**Verdict: accept.** `-mno-fma` emitting `vfmadd` is not an optimisation
disagreement; it is an illegal instruction on the machine the flags describe.
`-ffp-contract=off` is a source-semantics violation: the program asked for two
rounded operations.

**Fix.** `fma_transform_allowed(neon, fp_contract)` gates the arm;
`FpContract::c_language_default()` is `Fast`, so `-O2` output is unchanged.

**Pin.** `check_fma_gating.sh` contracts 1–3: four packed FMAs by default
(counted on `%ymm`, so the legitimate scalar contraction cannot be mistaken for
the packed arm), **zero FMAs of any width** under `-ffp-contract=off`,
**zero** under `-mno-fma`, and back to four under `-ffp-contract=fast` — the
last one so that "zero under `off`" cannot be a frozen counter.

**Non-vacuity, mutation-tested.** Pointed at a wrapper that silently strips
`-ffp-contract=off` from the argv, the gate fails with exactly the message it
was written to give:

```
  FAIL: -ffp-contract=off still emits 4 FMA(s)
check_fma_gating: FAILED      (exit 1)
```

A gate that cannot fail for the bug it names is decoration. This one was proved
to fire.

## 4. `P1-a` — `fold_shift_into_sib` and `reg_refs` — ACCEPT, severity reframed

**Claim.** The SIB fold scans some register-usage lists but not
`infos[j].reg_refs`, so a read of the index register can be missed; a later
dead-write pass may then delete the index's definition.

**Verdict: accept the defect, reject the implied severity.** The recon is
correct — the scan omitted a list that does record reads, and the invariant the
fold relies on ("no reader of the index remains") was therefore not actually
established. But I could not build a C-level reproducer: the shape that would
need to trigger it (`/tmp/sibreal.c`) is compiled identically correctly at
`-O1`, `-O2` and `-O3` on both the unfixed and the fixed tree, and matches
`gcc`'s output at every level. So this is a **latent contract bug**: the pass
asserted an invariant it had not checked, and the failure mode — a silently
deleted index definition — is the kind that survives review precisely because
it is silent. Latent is not harmless; it is unfalsified.

**Fix.** The fold now scans `infos[j].reg_refs` like the other lists.

**Pin.** `mod shift_into_sib_tests` (four cases) plus the existing
`check_rmw_sib_folds.sh` contract 3, which asserts the index's definition
survives in the emitted text, with a live control: the same gate asserts the
fold *fires* on the histogram shape and that the addressing mode is well
formed (every register operand carries its `%`, so the assembler cannot encode
the right bytes while the pass's own bookkeeping sees no read — the exact
failure mode this gate was written after).

## 5, 6. `P1-b` widened-mask width; `P1-c` flags horizon — ACCEPT

**Claim `P1-b`.** Widened-mask state is tracked in a byte, so bits are lost
past eight widened registers, and a mask test can then be wrong.
**Claim `P1-c`.** The compare/branch fold's `boundary` horizon was not
fail-closed: a flag reader past the horizon could be missed.

**Verdict: accept both.** Both are the same class as 4 — an invariant that is
asserted rather than checked. Neither has a reproducer I could construct, and
both are cheap to make unconditionally true, so "no reproducer" is not a reason
to leave them.

**Fixes.** `widened_mask: u16` with the tracking sites updated; the flags
scan now fails closed at its horizon.

**The first version of the `P1-c` fix was itself a regression, and an existing
unit test caught it.** "Fail closed at the horizon" was implemented as "refuse
whenever the bounded window closes", which also refuses when the window closes
because *the section ends* — there is nothing left to read the flags, so that
is not the absence of evidence, it is the presence of a boundary. The visible
effect was a missed fold on every small function, and
`tests::test_condition_codes` (whose input ends immediately after the jump)
went red in the full `cargo test` gate. The rule is now stated exactly: a full
flag writer **or the end of the scanned region** ends the hazard; only
exhausting the 64-line *cap* while code continues refuses. That is the sharper
version — strictly more conservative than the original fail-open code, and not
one line more conservative than the argument supports. The distinction is
pinned by `fuses_when_the_code_ends_inside_the_window` next to the refusal test
and its live control.

This episode is the argument for the full gate in one line: the fix was green
in every targeted test I had written by hand, and red in the suite I did not
write. **Residual, stated rather than hidden:** the walk is linear over text,
so it does not model control flow — a reader reachable only through a backward
edge is not covered by the window at all, in the original code or in this fix.
Closing that needs the per-path flag walk (upstream `57cd23ab`); this fix
neither introduces nor removes that limitation, and the note exists so the
limitation is not mistaken for a guarantee.

**Pins.** `mod widened_copy_tracking_tests` — a refusal case *and* a live
control, because a test that only asserts "no fold happened" passes on a
compiler that folds nothing anywhere; the control asserts the fold still fires
where it is legal. `mod flags_horizon_tests` for the horizon.

**On `P1-c` and upstream.** There is an upstream per-path flag walk
(`57cd23ab`) that would supersede this fix. The local fix is fail-closed and
strictly weaker than the walk (it refuses where the walk would prove), so it is
safe either way; when the walk lands, the local horizon should be deleted in
its favour rather than kept as a second opinion. Recorded here so the
supersession is not forgotten.

## 7. `P2` — affine exit-compare algebra — ACCEPT

**Claim.** The affine exit-comparison canonicalisation is unsound in the
flipped-orientation case.

**Verdict: accept, with an exact statement of the algebra.** The identity the
pass may use is

```
   N op (iv + C)   ≡   (N − C) op iv          (op unchanged)
```

The shipped code double-negated: it changed the operator while rewriting the
comparison in place, which is not the same statement — the two agree only when
the negation of the relation is taken *with* the operand flip, and it was being
taken alone.

**Evidence.** The broken version evaluated the true predicate `100 > (0 + 4)`
as `96 < 0` — false. That is not a near-miss or an edge case; it is the wrong
answer on an ordinary comparison.

**Fix.** The operator is left unchanged.

**Pin.** `mod affine_fold_orientation_tests` is an equivalence oracle over
>400 combinations of relation × orientation × `C` × `N` × `iv`: the folded and
unfolded forms must agree on every one. A single-case regression would have
been satisfied by the wrong fix; the oracle is not.

## 8. The kill switch that did not reach the clone — ACCEPT

**Claim.** `CCC_NO_AFFINE_EXIT_FOLD` does not disable the whole transform.

**Verdict: accept.** The environment check sat at one call site; the clone
fold ran through another. A kill switch that disables part of a transform is
worse than none: it makes a bisect lie. A transform that cannot be turned off
cannot be blamed or exonerated, and it is the primary tool for exactly this
kind of investigation.

**Fix.** The gate covers both.

**Pin.** `check_affine_exit_compare.sh` now asserts **zero** clone-fold reports
under `CCC_NO_AFFINE_EXIT_FOLD=1` while `[ROT] candidate:` markers are still
present in the same run — the second half is what makes the first half mean
something, because "no reports" is otherwise indistinguishable from "no
candidates".

## 9. Fused volatile read-modify-write — REJECT as stated

**Claim.** Fusing `volatile long h; h += 3;` into `addq $3, h(%rip)` violates
the `volatile` contract, because the read and the write are no longer two
separately observable accesses.

**Verdict: reject as stated.** The fused form performs **exactly one read and
one write of the object, in the source's order, with no access to any other
location in between** — the same count and the same order as the trio it
replaces. That is the observable contract `volatile` states for a
single-threaded observer, and the only one the standard guarantees: C11
§5.1.2.4p25 makes conflicting accesses to a non-atomic object from another
thread undefined behaviour whether or not it is `volatile`. What the fusion
changes is granularity — the read and the write become indivisible, so no other
agent can interleave. That is *stronger* than the abstract machine's guarantee,
never weaker, and it is not something a compiler is forbidden to do.

**Two pieces of hard data, one of which settles the design question.**

```
volatile long h;  void k(void) { h += 3; }
  lccc -O0 :  pushq %rbp ; addq $3, h(%rip) ; popq %rbp ; ret
  lccc -O2 :  addq $3, h(%rip) ; ret
  gcc  -O2 :  movq h(%rip),%rax ; addq $3,%rax ; movq %rax,h(%rip)
```

The fused RMW is present at **`-O0`**, before any peephole pass runs. So the
decision lives in the IR/isel layer, where volatility is visible, and *not* in
`fold_memory_rmw`. This pass matches assembly text, in which a volatile load is
indistinguishable from any other load: a volatility test there would be a claim
about information the layer does not have. The review's implied remedy — make
the peephole special-case volatile accesses — is not implementable at that
layer, and implementing it there would be exactly the kind of "proof by
pattern-match" that §7 and #11 in this document are about.

(The review's sibling observation is worth keeping: `volatile int g = g + 1;`
is *not* fused, because the read feeds a sign-extending load and an `int`-width
ALU op. So the behaviour is not "everything volatile gets fused"; it is the
`long`-sized RMW shape, decided once, above the peephole.)

**Fix.** The policy is documented at `fold_memory_rmw` with the evidence above,
including the residual, stated rather than hidden: a fused RMW is not a
substitute for `_Atomic`, and code that needs the read/write pair to be
interruptible by an ISR (a lock-free ring producer, an MMIO FIFO) must use
atomics — exactly as it must against GCC, which keeps the pair separate but
offers no ordering guarantee either.

**Pin.** `check_rmw_sib_folds.sh` contract 4: the volatile object must be
touched by **exactly one** instruction inside `k` (a redundant access is a
wrong-code defect — it can pop a FIFO or clear a status register twice), and
the program must still compute 13. The count is asserted inside the function,
not over the whole file, after the first version of the check counted `main`.

## 10. Unreferenced wrap probes and a dead conditional — ACCEPT

**Claim.** `wrap_affine_fold.c` / `wrap_loop_fold.c` exist but no gate runs
them; `wrap_loop_fold.c` contains a conditional whose body is empty.

**Verdict: accept.** Coverage that nothing executes is worse than missing
coverage, because it reads as done in a file listing.

**Fix.** `check_affine_loop_fold.sh` gained contract 4: both wrap probes run,
must print `exact`, and must print the *same* thing with the fold killed — the
second half makes the first meaningful. The dead conditional is gone (it
evaluated and discarded; the real comparison follows it).

## 11. A gate whose failure path could not fail — self-found, ACCEPT

Writing 9's regression exposed a defect in the gate itself. `bad()` sets
`fail=1`, and the `fail` test sat **before** the newly appended contract, so a
failure in it printed `FAIL` and then printed `PASS` and exited 0. The gate
could not fail. It was found by running it against source that *should* have
failed it, and it shrugged.

This is the same species as 4, 5 and 6 — an assertion that was not actually
checked — and it is the reason every new contract in this round is run against
a shape that must trip it before it is committed. The appends now sit before
the failure test, and the gate exits 1.

## 12. What the review did not file

- **The gate defect in §11.** Found by exercising my own new test against a
  hostile compiler wrapper. Reviewers cannot find this class without running
  things; it is listed here because the review's blind spot is the whole reason
  this round exists.
- **The refusal half of the matmul arm.** The review looked at the shape proof
  and asked whether it was complete; the sharper question is whether it is
  *exercised*. It was not: nothing pinned that a call in the body or a side
  exit keeps the packed arm off. Those are now contracts 4 in
  `check_fma_gating.sh`, counted on vector registers so the legitimate scalar
  contraction is not confused with a leak.

## 13. Verification state at this revision

All on the tree this document ships with, all reproducible from the repository:

| Check | Result |
|-------|--------|
| Envelope, N ∈ {16,17,31,32,33,48,49,64,65} × {`<`,`<=`}, vs `gcc -O0` | **19/19 exact** |
| Runtime inclusive bound `j <= lim`, N=17 | **4196.600000 == oracle** |
| FMA gating: default / `off` / `-mno-fma` / `fast` | **4 / 0 / 0 / 4** |
| `check_fma_gating.sh` (incl. mutation test) | PASS, and FAILS when mutated |
| `check_affine_loop_fold.sh` (contract 4 incl. wrap probes) | PASS |
| `check_affine_exit_compare.sh` (kill switch, non-vacuous) | PASS |
| `check_rmw_sib_folds.sh` (contracts 1–4) | PASS |
| Unit tests: `shift_into_sib`, `widened_copy_tracking`, `flags_horizon`, `affine_fold_orientation` | **10/10** |
| `check_env_test_hygiene.sh`, rustfmt, remaining shell gates | PASS |

Performance is unchanged by everything above: the packed arm still emits four
`vfmadd` at `-O2`, the affine folds still fire where they are legal (measured
1.1995× on `affine_countdown.c`, 1.078× on `affine_loop_fold.c`), and the only
output removed is output that was illegal.

## 14. Reproduce

Everything below is in the patch and in the default CI battery -- a probe that
only exists in a session directory is the defect in §10 with extra steps.

```sh
git apply ms178-1.patch

# the wide envelope (2 s): every size at which the remainder length changes,
# plus the FMA contract/target matrix, against gcc -O0
python3 tests/regression/verify_pr713_audit.py

# the pins: exact counts, the unproven-shape refusals, the kill switch, the
# volatile access count -- all part of the fast battery ci_local.sh runs
cargo test --profile fastbuild -j2 --lib -- \
    shift_into_sib widened_copy flags_horizon affine_fold_orientation
for g in check_fma_gating check_affine_loop_fold check_affine_exit_compare \
         check_rmw_sib_folds; do CCC=target/fastbuild/lccc bash tests/regression/$g.sh; done
```

`check_fma_gating.sh` and `verify_pr713_audit.py` are registered in
`scripts/ci_local.sh` (`fma-contract-gating`, `audit-envelope`) precisely
because the finding in §10 was about coverage nothing executes.

## 15. The design principle this round is really about

The findings all share one shape: a pass *asserted* an invariant it could not
observe, and nothing falsified it. A scan that missed a register list, a mask
narrower than the state it tracked, a horizon that did not fail closed, a
transform that could not be switched off, a fold whose volatile policy was
folklore, a gate whose failure path could not fail. None of these are exotic
algorithms; they are the difference between a compiler that is *demonstrably*
right and one that is *probably* right.

That is also where this compiler can be a magnitude better than the others
rather than a copy of them. GCC and LLVM encode these decisions as implicit
invariants spread across passes and hope the tests find the violations;
lccc now states them as named, checked contracts at the top of each fold (see
`fold_memory_rmw` and the matcher shape proof in `vectorize.rs`), pins the
observable consequence with a test that has been proven to fail, and keeps a
kill switch that reaches the whole transform so any suspicion can be bisected
to the pass that caused it. Matching the other compilers' instruction counts
is not the goal; being able to prove ours from the repository is.
