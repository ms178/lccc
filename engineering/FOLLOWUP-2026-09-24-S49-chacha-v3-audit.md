# Follow-up — S49: chacha20/ISA/audit session (2026-09-24)

Base: `07ac7c26` (ms178/lccc main, PR #605).  Deliverable:
`ms178-1.patch`.  Provenance: `engineering/PERF-PROVENANCE-S49.md`
(every number re-derivable; §1c pending the clean-base worktree
build — fill it before quoting base-vs-S49 deltas).

## A. Accomplished this session

1. **chacha20_block triage + fixes.**  Oracle-verified attribution
   (S47 did not regress chacha; GCC 16.2 scalar shape moved the
   reference side) + three landed improvements: `.LCVEC` const-pool
   rotate masks, SSE-128 chain copy-source homing (frame 0xd0→0x60),
   AVX-512VL `vprold` + pass-side mask bypass.  Measured: default
   1.036× gcc-14.2 scalar, `-march=native` 0.842× (all outputs
   bit-identical).
2. **Integer-v3 default ISA.**  `resolved_*()` completes the
   documented x86-64-v3 default (BMI1/BMI2/LZCNT/POPCNT/MOVBE) with
   explicit-`-march` ceiling + `-mno-*` denial tracking; macros
   follow codegen; `__POPCNT__` newly defined.  bitops −25.0%
   (interleaved idle A/B; −18.1% sequential screening),
   zero regressions (5-workload A/B, outputs identical).
3. **Review-AI audit work order, complete (P0+P1+P2).**  5 window
   predicate tests, 12-row andn agreement contract, multi-block
   majority-rank tests, single-sourced width rule, anchored ch-maj
   census + guarded shape scans, IVSR default-off differential pin,
   re-strengthened store_alu gate, acc-substitution register-class
   guards, sibling-consistency contract, knob registry, gate rename,
   wording fix, this provenance file.  Adjudication notes in §C.
4. **Oracle diagnosis bank** for the next phase: sieve (redundant
   imull ×3/iter), expat (hash spill vs callee-saved), chacha (ICX
   structure).  Exact shapes + re-runnable commands in §B and the
   provenance doc.

## B. TODO — ranked by (impact × workloads × confidence ÷ cost)

### B1. Sieve-class redundant multiply (HIGH value, MEDIUM cost)

`count_primes`: `i*i` computed 3× per outer iteration (LBB1 bound
test, LBB3 inner-start, LBB6 backedge test); GCC computes it once.
Shape: same i32 `Mul` of the same SSA value, the first dominating
the others across the `if (sieve[i])` diamond → textbook global CSE.
Investigate why GVN/redundant_loads missed it (suspects: the two
muls carry different IR types/flags; GVN runs before the diamond is
formed; or the backedge copy breaks value identity).  Repro:
`tests/benchmark/programs/sieve.c` + `godbolt.py compare … --function
count_primes` (gcc 34 vs lccc 47 insns).  Win: removes 2× imull
(3c latency each) per outer iteration; generalises to every
search-then-squared-index loop.

### B2. Callee-saved homing for call-live values (HIGH value, HIGH cost)

expat `main`: the scan hash is loaded+stored to its slot per
character; GCC homes it in %r12 across the `expat_utf8_name_length`
call.  The RA needs a "live-across-call ⇒ prefer callee-saved home"
policy (cost model: call frequency × spill traffic vs callee-saved
save/restore).  Repro: expat_xml_scan.c `main` (gcc 97 vs lccc 156).
Watch: must not pessimize leaf functions (no calls ⇒ no callee-saved
pressure).  Gate: new codegen-shape check on the expat scan loop +
   the benchmark-output oracle.

### B3. chacha → ICX structure convergence (HIGH value, HIGH cost)

Three rocks, in dependency order (ICX 74 vs lccc 132 insns; lccc
rotates already best-in-class):

1. **Copy-in forwarding (W5a, middle end):** the ARX pass knows the
   preheader copy loop `x[i]=in[i]` feeds the four packs — replace
   with direct `VecLoad` of the four lane groups from `in[]`.
   Legality: `in[]` must not alias `out[]` in a way the copy-loop
   deletion would change (it is read-only input; prove no intervening
   store to `in` between copy and vector use).
2. **Latch-copy elimination (W2b, allocator):** extend the
   destructive-chain handoff so the header phi and its latch value
   share one home (in-place loop, zero `vmovdqa`).  Highest risk of
   the three — touch only with the full battery + a new ARX latch
   shape gate.
3. **Feed-loop forwarding (W5b, middle end):** recognise the exit
   `out[i]=x[i]+in[i]` scalar loop and emit 4× (VecLoad in-group,
   VecAdd, VecStore).  Needs a small exit-loop recogniser in the ARX
   pass; legality = same no-alias proof as (1) + `x[]` dead after.

### B4. andn memory-operand fold (MEDIUM value, LOW cost)

find_bit scan: `mov; mov; andnq; test; je` (5/word) → fold ONE load
into andn's r/m (inverted) operand with an operand swap:
`movq b-word,%rax; andnq a-mem,%rax,%rcx` (4/word, matches GCC).
Work site: `emit_and_not_impl` + the generation-time fusion's
operand preparation.  Careful: the ch-maj gate pins the andn census
— extend, don't weaken.  Expected gain small (loop is L1-bound),
but it generalises to every andn-with-double-load shape.

### B5. IV widening follow-through (MEDIUM value, MEDIUM cost)

Sieve outer loop keeps a `movslq` per iteration; the IV-widening
pass exists (CCC_NO_IV_WIDEN) but didn't fire here.  Find the
rejection reason (likely the widened-IV legality proof vs the inner
loop's 64-bit use, or a cost-model veto) and close it.  Repro: same
as B1.

### B6. Quote-loop mem-operand compare (LOW value, LOW cost)

expat inner skip: `movzbl; cmpl; je` + counter maintenance vs GCC's
`cmpb (%rax),%r8b; jne`.  Small peephole/isel gap (memory-operand
compare selection for single-use loads feeding compares).  Do together
with a census pass over other mem-operand-compare misses.

### B7. Count-loop sbb idiom (LOW value, LOW cost)

Sieve count tail: 7-insn cmov vs GCC's 5-insn `cmpb/sbbl $-1`
branchless count.  A bit_idioms/isel pattern (`cnt += (x == 1)` →
sbb form).  Only after B1/B5 (itGetter: the tail is cold relative to
the marking loops).

### B8. sha256_transform loop-structure gap (MEDIUM value, TBD cost)

Oracle −O2 insns: clang 126, gcc 154, lccc 159 (artifacts committed
under the session's oracle-evidence dir).  The delta is NOT total
operation count — it is CONTROL: lccc carries ~2× the
branches/comparisons of clang (4× jl + 4× jge + 5× cmpq + 4× cmpl
vs clang's 2× jne + 2× cmpq whole-function).  Suspects: message
schedule + compression not fused/co-iterated the way clang lays
them out, or extra bound/versioning tests.  Needs a loop-by-loop
study with the committed artifacts; do NOT guess from the
histogram.  Note lccc already wins rotate selection (6× `rorx`
3-operand vs clang's 2-operand `roll`).

### B9. Pre-S47 attribution build — DONE this session

Closed mechanically: 93596224 vs 07ac7c26 `chacha20_core` asm is
byte-identical (0 diff lines), outputs identical.  The "S47
regressed chacha" hypothesis is disproven; the ratio moved on the
reference side.  See provenance §1a.

## C. Audit adjudication (for the next agent's context)

I agree with ~all of the Review-AI findings; the work order is fully
implemented.  Points of judgment worth preserving:

- **F4 (store_alu gate): partial pushback.**  The wiring pin was an
  honest, data-backed decision (inert-on-corpus measured 2026-09-23),
  not laziness — but the audit is right that the pin was one-sided.
  The completion (call-site COUNT = 2, sk()-gate parity, no-default-
  disable scan, residual hole stated in one paragraph) is the right
  shape for "pins a deliberately-inert pass".  Use it as the template
  if another pass goes inert.
- **F2's first fixture was wrong, instructively:** a SINGLE foreign
  And is soundly keepable (kept terms are read-only) — my initial
  "decline" test asserted an unsound expectation and the PRODUCTION
  code was right.  The landed pair pins both the two-foreign decline
  and the single-foreign mirror.  Lesson: when a new test disagrees
  with reviewed code, distrust the test first.
- **F7's guard is a belt, not a filter:** `!vector_values.contains`
  cannot fire today.  If it ever DOES fire in a debug build, that is
  a new bug (something cached an xmm value in the integer acc), not
  an expected event — add a debug_assert in that direction if the
  acc cache ever grows a writer.
- **F10's process gap is now closed by policy:** PERF-PROVENANCE-*
  docs are the price of admission for every perf claim.  The
  rbtree +1.2/+4/+4.3% numbers remain second-hand (quoted from the
  in-tree S46 post-mortem comment, not re-measured) — flagged as
  such in the provenance doc.

## D. Structural debt noticed (not scheduled)

- `MachInst` has two matches with opposite maintenance semantics by
  design now (P2-10 contract comment).  If a third consumer appears,
  consider a single `machinst_def_use(inst)` helper returning
  (defs, uses, touches_rax) instead.
- The `has_scalar_andn` permission is now single-sourced in
  `scalar_andn_available`, but the *emission* side
  (`supports_and_not`) still reads the env var directly — the P0-2
  contract test covers the fold/fusion agreement, not the two
  permission spellings.  A future cleanup could thread one resolved
  bool from driver → passes → backend; today the two spellings are
  verified equivalent by the ch-maj gate's kill-switch arm.
- The godbolt oracle cache (`.godbolt-cache/`) is git-ignored; the
  session manifests live only in /tmp.  Consider committing
  manifests for the golden kernels (chacha/expat/sieve/sha256) so
  the next agent starts from committed oracle evidence.
