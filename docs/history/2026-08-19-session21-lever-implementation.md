# 2026-08-19 (session 21) — session-20 lever implementation push (L1/L2/L4 shipped, L3 ceiling proven)

**Base:** `origin/main` @ `e47e82e` (PR #138 landed session-20's unary-RMW
classification fix + slot-RMW collapse — rebased onto it; local commits were
byte-identical to the squash, rebase auto-resolved)
**Snapshot:** `/home/user/ms178-1.patch`
**Mission:** implement ALL levers from session 20's roadmap; breakthroughs on
the 32 KiB kernel boot gate.

## Shipped (each fully validated — see validation ledger below)

### LEVER 4: Load ecx-hazard refinement (Phase 2d) — −200 B corpus
`collect_i686_scratch_hazard_points` treated every non-alloca Load as a %ecx
hazard because slot-resident pointers stage through %ecx. Two changes:
1. **Emitter** (`try_emit_load_direct`): register-resident pointers are now
   dereferenced DIRECTLY (`movX (%ptr),%d`) — no %ecx staging. Pure size win
   independent of allocation.
2. **Phase 2d wave** in `allocate_registers`: after all other phases, recompute
   hazards with actually-clean loads and hand ecx/edx to the values Phase 2
   had to refuse. Clean-pointer proof: every Load with the pointer must be
   gpr32 + dest-register-resident (guarantees the direct path) + pointer not
   an alloca; never-materialised (folded-absolute) pointers always clean.

**The fuzz caught TWO unsound drafts before the shipped version:**
- Draft 1 assumed register-resident pointers never stage %ecx — false,
  `emit_load_ptr_from_slot` copies reg→%ecx unconditionally (300/900 fail).
- Draft 2 re-entered caller-saved registers already handed out by Phase 2
  WITHOUT a holder-overlap check — Phases 1/2/2c are sound only because their
  pools are DISJOINT. A loop counter and its bound landed in %edx
  simultaneously; the bound's leal clobbered the counter (300/900 fail).
  Fixed with an explicit holder-interval conflict filter.
Lesson: re-entering an allocation pool needs the same interference proof as
the primary scan. Env gate `CCC_NO_LOAD_HAZARD_REFINE` for bisection.

### LEVER 1: accumulator-flow for immediately-consumed values — −592 B boot objects
`immediately_consumed` previously only denied SLOTS; the allocator still gave
register homes, so consumers staged `movl %R,%eax` relays anyway. On i686
EVERY immediately-consumed consumer reads via `operand_to_eax` (acc-cache
first): Store val, Cast/UnaryOp/Copy src, BinOp/Cmp lhs, Return operand.
Now they get NO home at all — the producer leaves the result in %eax with a
live cache entry and the consumer hits the cache (zero instructions).
strlen's tail became GCC's exact shape:
`movl %esi,%eax; subl %ebx,%eax; ret` (2 pushes instead of 3).
Mechanics: the set is merged into `never_materialized` in the i686 prologue
(state.never_materialized_values — the emitter fold set — deliberately NOT
touched; these values DO emit their producer).
**Enabled by LEVER 1b (is_sole_operand_of_terminator):** Return is now a
fused consumer on 32-bit targets (the return path IS operand_to_eax; wide /
float returns fail the producer gate earlier).
**EXCEPTION (found via check_load_widen_cast_no_relay):** CondBranch
conditions keep their homes — a register-resident cond tests in place
(`testl %R,%R`), a home-less cond pays `movl %src,%eax; testl %eax,%eax`
because no-op-coalesced producers set no acc-cache entry.

### LEVER 2: CFG-aware phi-web coalescing machinery — sound, 0 boot delta
Implemented the session-20 design: phi-transport Copy edges marked
same-value (dest feeds a Phi / src is a Phi dest), and phi-congruence
CLASSES (dest + all incomings of each Phi, union-find) accepted as
same-home in the overlap check even with overlapping linear intervals
(mutually exclusive CFG paths — the classic web argument).
**Measured result: zero corpus/boot delta.** Root cause established (this is
the durable finding): the boot corpus's dominant slot traffic is NOT web
duplication — cpucheck/cmdline's long-lived values are DISTINCT variables
(state, c, op, w…) with true simultaneous liveness (~10+ values at the loop
header vs 2 caller-saved + 4 callee-saved registers). GCC wins via
TER/out-of-SSA + instruction-level flow that never materialises most of them,
not via web merging. Machinery kept (correct, fuzz-clean; may pay on
switch-heavy non-boot workloads), lever re-scoped in the roadmap.

### LEVER 3: %eax allocatability — ceiling PROVEN, implementation deferred
Ceiling measured on the current corpus: 391 eax relays total (178 in + 213
out ≈ 0.8 KB upper bound if ALL vanished) vs 2069 slot refs (~8–10 KB).
Ultra-conservative hazard whitelist (the only sound variant without emitter
rework: Copy/Cast/Load/Store/BinOp/Call all clobber %eax on their
accumulator paths) leaves almost nothing eax-clean, so a flag-gated prototype
would measure ≈ 0. The real lever requires the emitter to tolerate a live
allocated %eax everywhere (operand_to_eax/store_eax_to fallbacks) — a
multi-session rework. Documented, not stubbed (§69: no dead weight).

## Gate status after session 21
boot object text **32 858** (session start 33 450: −592). Budget ≤ **23 330**
(pecompat must fit at 0x6000; bss 4 915 fixed). Gap **≈ −9.5 KB (−29 %)**.
GCC reference 12 846.

## Remaining gap — ranked, evidence-backed (next sessions)
1. **SIB-indexed absolute addressing** (`sym+disp(,%idx,4)`) for
   GlobalAddr+GEP chains — GCC's `cpu+12(,%ecx,4)` one-instruction array
   access vs lccc's materialise-base + imull/shll + addl (~10–15 B vs 4–7 B
   per site; 38 materialisations + 40 add-chains in the boot corpus alone).
   ISel feature: fold GEP(GlobalAddr, idx, scale) into one memory operand;
   also enables memory-source/dest ALU on globals (`andl req_flags(,%ecx,4),%eax`).
2. **Materialisation-policy allocator** — the true GCC gap: values that flow
   producer→consumer within an expression chain should never receive homes
   (extend the immediately-consumed idea across multi-instruction windows
   with CFG-aware cache invalidation; equivalently TER at the IR level).
   The 2069 slot refs are the prize (~8 KB).
3. **Memory-destination ALU with liveness proof** — `incl S` class folds
   where the result is dead or register-consumed afterwards (the RMW collapse
   shipped this session covers the dead-after cases; live-result cases need
   the allocator to prefer acc-compute + direct store).
4. **%eax allocatability** — only after the emitter tolerates live %eax
   (ceiling ~0.8 KB; sub-priority to 1–3).

## Validation ledger (session 21 final, all on the rebased tree)
- unit: 957/957
- correctness: 50/50
- regression --compare-gcc: 351 + 6 known gcc-14 oracle mismatches
- m32 differential fuzz: 900/900 (0:300 × O0/O2/Os)
- slot_rmw differential fuzz: 750/750 (0:250 × O0/O2/Os)
- boot pipeline: 23 objects + lccc-ld setup.ld asserts faithful
