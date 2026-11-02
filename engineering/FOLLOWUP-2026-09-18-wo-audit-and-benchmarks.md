# Follow-up: red-team audit WO-1..8 + the worst-15 benchmark campaign

Session: 2026-09-18 (session 55, post-PR-#554)
Base: `ms178/lccc` main @ `11e3a1cd` (tree-identical to the merged S27
snapshot `21d8448a`; verified `git diff` empty before any work started).

## 1. The audit verdict (WO-1..8) — implemented, with corrections

The Review AI's audit was strong: every work order identified a real
defect or gap, and its "do NOT" list is correct (verified against the
current code). Two factual corrections were needed before implementing:

* **F4's rationale is wrong even though its conclusion is right.**
  `vpunpckldq` and `vpinsrd` DO have VEX encodings; the seed path's
  legacy `movd`/`punpckldq` is SSE2-exact *by design* ("no pinsrd
  dependency" — the emission comment) so the pair packs on every
  x86-64 baseline. The WO-5 wording fix states the true reason, not the
  audit's false one.
* **WO-5's `ci.yml` site does not exist** — the overstated "no
  legacy/VEX mixing" claim lives only in the red-team script header; the
  ci.yml comment ("VEX in-place forms") is accurate. Only the script
  was reworded.

| WO | What landed | Verification |
|----|-------------|--------------|
| 1 | `remove_dead_leaf_frame` classifies epilogues by **bidirectional ret-pairing** (an `addq $N,%rsp` is an epilogue iff the next real line, skipping NOPs/directives, is a `ret`); mid-body spellings set the decline flag. Balance proof in the comment: every entry→ret path crosses exactly one prologue subq and one ret-paired addq. | 8 new whole-pipeline unit tests (`frame_compact_tests.rs`): 3 positives (incl. two-epilogue and directive-separated), 5 negatives (mid-body addq, label-separated addq, unpaired ret, `leaq 8(%rsp)` observer, alignment-frame mid-body addq). The two pre-existing `regression_tests` frame_compact pins still pass (no behavior change on accepted shapes). |
| 2 | The CFG rewire's `CondBranch` arm now has `else { return false; }` + a `debug_assert!` pinning the invariant (`body_entry == latch_label` ⇒ a terminator arm is the latch) right after the checked precondition. | Full unit suite (2901) + battery green; behavior unchanged on all accepted shapes. |
| 3 | `CCC_NO_LOOP_VEC_MAX=1` kill token on the Max-reduction gate; 6th red-team differential leg; **the measurement landed**: find_max, 10M i32 / 40 MB, 15 sweeps, paired medians of 9 interleaved runs: **vec 53 ms / scalar 134 ms / gcc 84 ms → vec/scalar = 0.396, vec/gcc = 0.631**. The v12-era "~1.4× slower" claim is decisively falsified (the scalar cmp+cmov dependency chain binds, not DRAM). The lift stands on the number. | Red-team gate green incl. leg 6; transform verified firing (60 `vpmaxsd` with gate on, 0 with token). |
| 4 | `-fsyntax-only` was **not implemented at all** (silently dropped by the unknown-argument handler on every path — worse than the audit knew). New `CompileMode::SyntaxOnly`: preprocess + lex + parse + sema per C input; assembly inputs gate through the preprocessor; objects skipped; no output, no link, `-o` ignored; `-Werror` promotion honored. The i686 header gate's probe restored from `-E` to `-fsyntax-only` (strictly stronger: parses the TU). | `printf '#include <stdio.h>\n' \| lccc -x c - -fsyntax-only` → 0; syntax error → 1 with diagnostics; missing header → 1; `LCCC_SYSROOT=… lccc-i686 -m32 -x c - -fsyntax-only` → 0. |
| 5 | Script header states the SSE2-exact seed design with the corrected rationale (above). | — |
| 6 | Both asm ranges (d4, f1) are bounded by `.size` **or the next function label** (`scoped_fn_asm`). | Negative test: hiding `.size` from a local dump → old awk captured 2201 lines (the false-pass inflation), new awk 239. Current-CI output identical. |
| 7 | `debug_assert!` pins the MemLoad arithmetic-progression invariant at the stream-CSE use site. | Debug builds of the suite; no change. |
| 8 | `free -m` fallback: `sysctl -n hw.memsize` / `hw.physmem` (bytes → MB), non-numeric-safe. | — |

## 2. The worst-15 campaign — root causes (measured, not guessed)

Method: paired lccc-vs-gcc medians locally (Xeon sandbox, 9 interleaved
runs each) on the Review CI's own kernels; asm-level diffing of the hot
functions. Local ratios reproduce the CI's ordering for every gap except
struct_copy (lccc wins locally, loses 1.17× on CI's EPYC — the stack
round-trips cost more there, same root cause).

### 2.1 The cross-cutting finding: the x86-64 register budget

GCC's round loop uses **15** GP registers. lccc's allocator pools cover
**13–14**: `rax` (accumulator scratch), `rcx` (secondary scratch — *in
no pool at all*), `rdx` (conditionally admitted), `rbp` (emitter scratch
when not FPO-homed). An exhaustive emitter audit (see §3) found rcx is
written by ~30 distinct emitter paths — the generic binop fallback
stages every non-imm32, non-register-homed rhs through it
(`movq slot, %rcx; op %rcx, %rax`), the peephole then folds the code
back to `op slot, %rax` *after* the RA already decided homes.

Victims (all measured): sha256's round loop spills a temp + (formerly)
a loop-carried web (4 stack ops/round vs GCC's 0); find_bit's scan pays
staging copies; zstd rematerializes `leaq buffer(%rip)` twice per outer
iteration. Every one of these is a register-budget symptom.

### 2.2 sha256_transform (CI 1.64×, local 1.28×) — three layers

1. **The schedule loop is at parity** (the d4 half-wide pair form; ~24
   insns/2W vs GCC's 25).
2. **The round loop's IR is now leaner than GCC's**: this session's
   boolean-algebra canonicalization (§2.3) rewrites CH to the 3-op
   `z ^ (x & (y^z))` — GCC keeps CH at 4 ops with a NOT — and MAJ to the
   4-op `(x&y) ^ (z&(x^y))` (GCC parity), and eliminates the old
   loop-carried (a&b) web entirely (round-loop stack ops 5 → 2).
3. **The codegen cannot express the win yet**: the remaining gap is the
   scratch model — every rotate stages through `movq %rXX, %rbp`
   (5×/round), the K base is rematerialized per round
   (`leaq K(%rip), %rcx` — the remat policy trading a register under
   pressure), one temp still round-trips `12(%rsp)`, and the 8-value
   state rotation materializes 8 `movq`s (GCC: 6; the theoretical
   coalesced minimum is 0 via phi-cycle register rotation — GLA P0-B).

Runtime: local 1.276 → 1.266 (the IR win is real but eaten by the
scratch tax). Full parity needs §3.

### 2.3 Boolean mux/majority algebra (NEW, landed, default-on)

`bit_idioms::match_bool_mux_algebra` — three bit-exact identities:

```
(x&y) ^ (x&z)        →  x & (y^z)            3 ops → 2
(x&y) ^ (~x&z)       →  z ^ (x & (y^z))      4 ops → 3   [SHA-256 CH]
(x&y) ^ (x&z) ^ (y&z)→  (x&y) ^ (z&(x^y))    5 ops → 4   [SHA-256 MAJ]
```

with Or-spellings where the identity holds (mux arms disjoint; maj
all-Xor and all-Or and inner-Xor/outer-Or — **inner-Or/outer-Xor is a
different function and is rejected**). Soundness discipline: producers
are repurposed in place under single-use proofs; every operand newly
introduced into a repurposed producer is dominance-checked (in-block
index order; cross-block defs dominate the whole block by SSA); v1
restricts repurposes to the consumer's block. **The coherence
contract**: rewrites (consumer AND producers) apply immediately with
`defs`/`use_counts` maintained in the same step — the first version
deferred producer rewrites and Pattern A + stale-defs Pattern C composed
into a miscompile (sha256 wrong digest, caught by the known-vector
check, root-caused via CCC_DUMP_EACH_PASS). 14 unit tests include an
exhaustive truth-table oracle (6³ multi-bit patterns per shape) and
decline pins (multi-use producer, unshared selector, cross-block
producer, late-defined new operand). Kill switch:
`CCC_NO_BOOL_ALGEBRA=1`.

Corpus validation: full suite 731/0/8, oracle 204/204, all gates green.
Blast radius measured: only sha256_transform changes among the
benchmarks (CH/MAJ shapes); outputs byte-identical everywhere.

### 2.4 linux_find_bit (1.27×) — three codegen gaps

GCC's word-scan loop is 4 insns/word (load, not, and-with-folded-load,
`jne` off the AND's ZF). lccc's is ~8: (a) the `and` does not fold the
non-Not side's load as a memory operand through the `~b` chain;
(b) the loop bound recomputes `leaq 1(%r10); shlq $6; cmpq` per word
instead of GCC's decoupled bit-counter (`leaq 64(%rsi)` + separate
compare); (c) the exit test is a separate `testq %r8,%r8; jne` at the
loop head instead of reusing the `and`'s flags (flag fusion across the
backedge).

### 2.5 zstd_count (1.22×) — tail inefficiencies

The 8-byte scan loop itself is competitive (5 insns/8B, xor folded,
flags reused). The gaps: `__builtin_ctzll` keeps its zero-guard
(`testq; jnz; movl $64`) even when dominated by the `jne` that proved
the operand nonzero (GCC emits bare `bsfq`); a double sign-extension
(`cltq` then `movslq`); and two `leaq buffer(%rip)` rematerializations
per outer iteration (the same remat-under-pressure policy).

### 2.6 struct_copy (CI 1.17×) — SROA of inlined struct traffic

Every FP temporary of the inlined `make_group`/`particle_distance`
round-trips a frame slot (`vsubsd → movsd %xmm0, 40(%rsp) → movsd
40(%rsp), %xmm0 → vmulsd`), while GCC keeps the values in xmm registers
and packs pairs with `movhpd/movlpd`. Root cause: by-value struct
arguments and struct returns through memory are not SROA'd after
inlining, so the lowered IR materializes every field through memory and
the scalar-FP homing follows. (The minimal `dx=a-b; dy=…` case IS
register-homed — this is specifically the aggregate-traffic shape.)

### 2.7 expat_xml_scan (1.31× local)

Prior session's CALL_ARGS liveness work already recovered −3.25%. The
remaining gap is branch structure (insn counts are near-parity:
188 vs 183): the classification tables need the branchless
select/cmov forms — the same cmp+blendv/select work as
aarch64_select_patterns (§4 follow-up item 4).

## 3. The rcx admission design (the structural fix)

**Goal**: bring the allocatable GP budget from 13–14 to 14–15 by
admitting `%rcx` (PhysReg 7 — currently `phys_reg_name(7)` is
`unreachable!()`) as a caller-saved home with per-value hazard
exclusion, mirroring the proven rdx Phase-2x64 wave.

**The exhaustive hazard map** (from a full emitter audit; every
CLOBBER-UNPAIRED site listed with its IR-reachable trigger):

* IR-exact classes: variable shifts/rotates (non-Const rhs — operand_to_cl);
  div/rem (divisor staged in rcx); BitTest variable index; FP Neg F64
  (sign-flip movabsq+xorq); AtomicRmw + AtomicCmpxchg (address staged in
  rcx); Switch terminators (PIC tables always, wide/min/range forms
  conditionally); VaArg/VaStart; calls (arg #4 = rcx; dyn-align and
  non-multiple-of-8 byval staging before the call point);
  inline-expanded memcpy/memset (rcx = rep count / loop counter);
  U64↔FP casts (the ≥2^63 paths); F128/long-double loads/stores/cmps
  and LDFabs/LDCopysign; the H19 intrinsic list (Movnti, Crc32,
  BuiltinLongjmp, DoBuiltinApply, Aes*256, Vpclmulqdq256, FmaF64x4,
  LoadF64x4/x2, LoadI32x8/x4, VecMulI64x2, VecPackI32x4, Rdtscp,
  BroadcastLoadF64); nested-function trampolines; the whole i128 body
  class (prep stages rhs low word into rcx — keep the
  `x86_body_has_wide_ops` whole-function gate).
* **Emission-dependent classes** (the hard part): the generic binop
  fallback (any Add/Sub/Mul/And/Or/Xor whose rhs is not imm32, not
  GPR-homed, and whose load-fold did not apply), integer Cmp staging,
  Select arm staging, and every Load/Store/GEP whose pointer resolves
  Indirect/OverAligned/Reg-default (slot-based or fold-refused bases).
  **These cannot be collected from the IR alone** — the fold decision
  (`can_const_addr_fold` / `can_indexed_addr_fold` / load_op_fuse) is
  emitter-internal, and the peephole erases the rcx traffic after the
  RA has already committed.

**Prerequisites** (in order):
1. **A shared fold-decision oracle**: extract
   `would_fold(load/store/gep, homes)` from the emitters so the RA can
   evaluate emission-dependent hazards exactly. This kills the
   "emitter and allocator disagree" class for rcx the same way the rdx
   doc did for rdx.
2. **Fixpoint wave**: assign rcx to overflow intervals, recompute the
   hazard set under the tentative homes (admitting an rcx home can
   REMOVE hazards — an rcx-homed rhs stages for free — and CREATE them
   — an rcx-homed GEP base loses const-offset folding, so exclude GEP
   bases or model the fallback), iterate to a monotone fixpoint.
3. **phys_reg_name / fold-refusal / sec-cache integration**: add the
   id-7 arm to the name tables, add 7 to
   `const_offset_fold_reg_base_ok`'s exclusion set (with rdx/r11), and
   make every rcx-home assignment invalidate the `sec` register cache
   (the audit's §4.11 contract: the cache and the pool must never both
   believe they own rcx).
4. **Verification targets** (pre-registered): the historical
   bitops_builtins `(h^v)*K` rcx miscompile, `__atomic_fetch_and`,
   cmpxchg, gzip/SQLite MachInst corpus, i128 torture, switch with
   64-bit out-of-i32 cases, sha256/find_bit/zstd runtime deltas.

Expected payoff: sha256 round-loop parity (no spill, remat pressure
gone), find_bit's staging copies, zstd's remats — and +1 register for
every function in every benchmark.

## 4. Priority order (next sessions)

1. **rcx admission** (§3) — the single highest-leverage item; the
   hazard audit above is the de-risking deliverable.
2. **Flag fusion across the backedge** (find_bit's `and`→`jne`; the
   `test`-after-`and` at loop heads) + **memory-operand folding through
   the Not chain** (`a & ~b` with `b`'s load folded, `a` in-register).
3. **struct_copy**: SROA of inlined by-value struct args/returns (the
   aggregate_sroa pass exists — investigate why the
   make_group/make_particle shapes decline).
4. **cmp+blendv packs / select canonicalization** (aarch64_select 1.16×,
   expat 1.31×, bitops) — the §4 item 4 from the SLP follow-up list.
5. **Phi-cycle register rotation** (GLA P0-B): the sha256 state
   rotation at 0 movs instead of GCC's 6.
6. **ctz/clz zero-guard elimination under a dominating non-zero branch**
   (zstd) + the `cltq`+`movslq` double-extension peephole.
7. The remaining §4 SLP follow-ups (128-bit VEX load folding design is
   in FOLLOWUP-2026-09-16 §3.1; dead frame slots for unhomed vectors;
   FP Neg; adler32 DO8).

## 5. Measurement appendix (this session, local sandbox)

| benchmark | lccc→gcc (pre) | lccc→gcc (post) | notes |
|-----------|----------------|-----------------|-------|
| sha256_transform | 1.276 | 1.266 | bool-algebra: −7 insns, round-loop stack ops 5→2, carry web gone |
| find_max (WO-3) | — | vec/scalar 0.396, vec/gcc 0.631 | gate lift validated |
| all other worst-15 | — | unchanged | bool-algebra blast radius = CH/MAJ shapes only |

Full battery at the snapshot commit: cargo test --lib **2901/0**
(2887 + 14 new), regression suite **731/0/8** AB-diff 0 (3-env, i686
sysroot + runner), benchmark oracle **204/204**, `ci_local.sh --fast`
**44/0 ALL GATES GREEN**, rustfmt green, clippy (lib/bins/tests,
-D warnings) clean, zero build warnings.

### 2.6-addendum (confirmed post-snapshot)

`CCC_DUMP_EACH_PASS` on `make_group`: the inlined `make_particle`
materializes a fresh 48-byte alloca per call, stores every field into
it, then `Memcpy`s it into the caller's slot (the sret-return path);
`g.particles[i] = <that>` adds another 48-byte copy, and each
by-value `particle_distance(a, b)` argument two more. **5 `Memcpy`
instructions survive to the FINAL IR (iter=2, post-ipcp)** — the
aggregate SROA / copy-forwarding chain (`aggregate_sroa`,
`aggregate_copy_forward`) does not collapse the
alloca→memcpy→slot→memcpy chain that GCC's SROA eliminates. The fix
target: sret forwarding through inlined struct returns + by-value
argument promotion, so the field stores land directly in the
destination and the FP values stay register-homed.
