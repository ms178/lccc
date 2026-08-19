# 2026-08-19 (session 22) — LEVER 3 shipped: %eax as an allocatable home (+ regparm fuzzer)

**Base:** `origin/main` @ `1949267` (PR #139 landed session-21's lever push)
**Snapshot:** `/home/user/ms178-1.patch`

## LEVER 3: %eax allocatability — implemented properly, shipped

Session 21's ceiling analysis (~0.8 KB relay bound) assumed the emitter could
not tolerate a live %eax. The shipped design makes it sound WITHOUT touching
the accumulator-staging paths, via the existing hazard machinery:

1. **`collect_i686_eax_hazard_points`** — whitelist scan: ONLY `Phi`
   instructions and `Branch`/`Unreachable` terminators are eax-clean; every
   other emission point (all ALU/memory/cast/copy/call/GEP/GlobalAddr,
   `Return`/`CondBranch`/`Switch` included) is a hazard. A value homed in
   %eax therefore only spans straight-line phi/branch corridors.
2. **Def-point exclusion** — a hazard at the value's own definition point is
   the value being BORN (producers route through %eax and leave the result
   there; `store_eax_to` on an eax home keeps the cache entry), not a
   clobber.
3. **Last-use shape gate (`acc_first_uses`)** — a hazard exactly at the last
   use is only safe when the consumer reads the value from the accumulator
   BEFORE reusing it: BinOp/Cmp LHS, Store val, Cast/UnaryOp/Copy src, Load
   ptr, Phi incoming, Return/CondBranch operand. A binop RHS is read AFTER
   the LHS is staged through %eax — the first draft homed an RHS in %eax and
   emitted `xorl %eax,%eax` (zeroed the accumulator; m32 fuzz seed 0 caught
   it instantly).
4. **Emitter no-ops** — `operand_to_eax`/`store_eax_to` on an eax home emit
   nothing (cache refresh only); `phys_reg_name(6)="eax"`; regparm capture
   self-moves already skipped by the parallel-move resolver.

**Numbers:** corpus 31413 → 31215 (−198 B); boot objects 32858 → **32576**
(−282 B). Full gauntlet green: 957 unit, 50/50 correctness, 351+6 regression
(same known gcc-14 set), 900/900 m32 fuzz, 750/750 slot-RMW fuzz.

## New infrastructure: regparm differential fuzzer

The boot corpus is `-mregparm=3`, which NO existing fuzzer covered (the m32
fuzz drives stack-arg cdecl). `tests/fuzz/regparm_differential.py` passes
seed in %eax / iter in %edx through the same fold-oracle harness:
**450/450** (0:150 × O0/O2/Os) with lever 3 enabled. (An ad-hoc regparm
sweep first showed 436 "mismatches" — root cause was the harness driving a
regparm probe with stack args; identical failures with lever 3 disabled
proved it was the harness, not the compiler. The committed script uses the
correct ABI.)

## Gate status
boot object text **32 576** vs budget **23 330** → gap ≈ −9.2 KB.

## Next (session 22 continues)
- SIB-indexed absolute addressing (`sym+disp(,%idx,4)`) — roadmap #1.
- Cross-backend audit/port: x86-64 equivalents of the i686 levers
  (immediately-consumed no-home flow, redundant-test elision, unary-RMW
  classification) where the MachInst backend lacks them; and i686 adoption
  of anything valuable x86-64 already has.

## Remaining-gap root-cause analysis (session 22 close-out)

Re-measured per-file gap (lccc − gcc, boot -Os): printf +2899, string +2280,
video +2021, cmdline +1186, cpucheck +1144, early_serial +1142. Function-level
in printf: vsprintf +1586, number +1166. Instruction census on the 6 largest
files: **movl 1593 vs 379 (+1214), esp-slot refs 1237 vs 179 (+1058),
movz/sbl 166 vs 52 (+114), popl 158 vs 65**. The residual gap is dominated by
**per-SSA-value materialisation** — lccc homes nearly every intermediate to a
stack slot where gcc keeps it in a register across an expression chain.

Three concrete shapes identified for the next session (all inspected, none
yet implemented — each is a distinct, bounded change):

1. **Indexed/SIB global addressing** (`sym+disp(,%idx,4)`). Constant-offset
   globals ALREADY fold (`arr+12`, `movl $7, arr+20` verified). Indexed
   accesses instead emit `movl $arr,%r; shll $2,%i; leal (%r,%i),%p;
   movl %p,%ecx; movl (%ecx),%eax` (5 insns + a relay) where gcc emits ONE
   `movl arr(,%ecx,4),%eax`. Root cause traced: the GEP pointer is not seen
   as register-resident by `try_emit_load_direct` (falls through the
   `%ecx`-staging branch), so the SIB/`(%base,%idx)` memory operand is never
   formed. Fix = fold GEP(base, idx, scale) into the Load/Store/ALU memory
   operand (SIB byte), and make the GEP result's register home visible to the
   load. GCC wins ~4–8 B per site × dozens of sites.

2. **Select (ternary) materialisation.** 77 `.Lsel_true/.Lsel_end` diamonds
   in the 6 largest files. Each `?:` becomes a branch diamond whose result is
   stored to a slot then reloaded. -march=i386 forbids cmov (gcc uses none
   either), so the win is keeping the select result register-resident /
   folding single-use selects into the consumer, not cmov.

3. **Materialisation-policy allocator** (the structural prize, ~8–10 KB).
   Values used once within an expression chain should flow register→register
   (or accumulator) without a slot home. This is the TER/out-of-SSA gap that
   accounts for the bulk of the +1058 slot refs. Higher risk; needs the
   immediately-consumed idea generalised across short multi-instruction
   windows with CFG-aware acc-cache invalidation.

## Cross-backend porting status (user directive)
Audited x86-64 for the session-21/22 levers:
- **Lever 1 (immediately-consumed no-home):** the x86-64 EMITTER already
  supports it — `store_result`/`store_eax_to` have an
  `immediately_consumed` branch that skips the store and relies on the acc
  cache (emit.rs:1400/1488). What x86-64 does NOT do is add those values to
  `never_materialized` for regalloc, so they still get a register home and
  pay a `movq %rax,%reg` + reload. Porting = add the (CondBranch/Return-
  excepted) immediately-consumed set to the x86-64 `never_materialized`.
  Expected gain small (registers are plentiful, no push/pop on caller-saved)
  and RISK is elevated (host backend + `call_arg_regs`/`indirect_target_regs`
  interactions). Deferred deliberately; recipe recorded here for a session
  that can give it full fuzz coverage.
- **Lever 3 (%eax home):** x86-64 has 16 regs; the accumulator-home idea is
  i686-specific (few registers). Not applicable.
- **i686 adopts from x86-64:** `redundant_ext`, memory-fold `fam_read_after`,
  and the direct-load/store bypass were ALREADY ported to i686 in earlier
  sessions. No further x86-64→i686 gap found this session.

## Validation ledger (session 22 final)
- unit 957/957 · correctness 50/50 · regression 351 + 6 known gcc-14 set
- m32 differential fuzz 900/900 (0:300 × O0/O2/Os)
- slot_rmw differential fuzz 750/750
- NEW regparm differential fuzz 450/450 (0:150 × O0/O2/Os) — covers the
  `-mregparm=3` kernel-boot ABI previously untested
- boot objects text 32 576 (session start 32 858; −282)
