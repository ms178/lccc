# 2026-08-20 (session 26) — the two remaining -O2 segfaults: root-caused and fixed

**Base:** `origin/main` @ `ce906b67` (PR #143 — session-25 fixes merged).
**Deliverable:** `/home/user/ms178-1.patch` re-based on this base (DAUERAUFGABE).

## Fixed defect 1 — zlib-ng gen_bitlen state corruption (all opt levels,
## non-compat AND compat): `sqlite3KeyInfoAlloc`-class lost store

**Symptom:** zlib-ng -O2 (previously also surfaced as the sqlite -O0… no —
this one is the *value-fabrication* class): `p->aSortFlags = (u8*)&p->aColl[N+X]`
stored NULL; the store emitted `xorl %eax,%eax; movq %rax, 24(%r13)`.

**Root cause chain (verified instruction-by-instruction):**
1. The val-side address `V = &p->aColl[N+X]` is a no-home immediately-consumed
   value: `emit_leaq_base_index_impl` staged it via `leaq (%base,%idx),%rax`
   + `store_rax_to(V)` — which correctly marks the acc cache `acc = V`.
2. `emit_leaq_base_index_impl` THEN ran its own `invalidate_acc()` —
   **destroying the handoff it had just established** (ordering flaw:
   invalidate must precede the cache-setting store, never follow it; the
   i686 override has the correct order).
3. The const-offset-folded store consumer then staged the val via
   `operand_to_rax(V)`: cache miss → no register home → no slot → the
   **silent `xorl %eax,%eax` fallback fabricated zero** and stored it.

**Fix (3 parts):**
- `emit_leaq_base_index_impl`: invalidate only in the register-dest branch;
  the acc-staged branch keeps the `store_rax_to` handoff intact.
- x86-64 `operand_to_rax`: the no-home/no-slot/no-cache fallback is now a
  **hard panic** (with value id + function name) instead of fabricating
  zero — a live operand in that state is a proven broken handoff, and
  silent zero is a miscompile factory (it hid this bug and the
  sqlite3VdbeExec one below for months).
- i686 `operand_to_eax`: same audit; the equivalent arm EMITS NOTHING
  (stale-accumulator class). i686's acc-cache contract is not complete
  enough for the hard gate yet (legit eax-only flows exist without cache
  entries — the gate panicked 10 regression tests), so it keeps the legacy
  no-op, documented as the **i686 acc-cache audit** open item. On x86-64
  the gate is fully validated (959/50/352+6 green, both workloads fixed).

**Repro (fixed):** `/tmp/krep.c` pattern — struct with flexible array,
`p->sort = (u8*)&p->coll[N+X]` followed by memset; at -O2 lccc stored NULL.

## Fixed defect 2 — sqlite3 -O2 SIGSEGV in `sqlite3KeyInfoFromExprList`

**Symptom:** `mov %al,(%rcx)` through NULL — `pInfo->aSortFlags` read back
NULL inside the KeyInfo loop (defect 1's store side contributed at -O0; at
-O2 a second, independent hole stranded the loop condition entirely).

**Root cause (verified with per-pass IR dumps + targeted backend tracing):**
sqlite's i128 multiply-overflow check lowers to
`Cmp{ty:I128} → Cast{I64→I32} → CondBranch` in one block.
- The prologue flag-fusion scan treats the chain as fusable
  (`cty.is_integer()` includes I128): it inserted the Cmp into
  `fused_cmp_dests` and the Cast dest into `fused_forward_dests`.
- The EMIT side routes I128 compares to `emit_i128_cmp`
  (`emit_cmp` dispatch) — which has **no fusion hooks at all**: it never
  sets `pending_cmp`, and the Cast is skipped anyway (fused_forward).
- The CondBranch found no pending flags and tried to materialize the
  chain-end boolean — which was never emitted → pre-gate: silent garbage
  branch (the observed SIGSEGV); post-gate: hard panic pointing straight
  at the stranded value.
- Same latent hole in the `cmp_replay` scan (would have re-emitted an i128
  compare through `emit_int_cmp_replay_insn` — wrong codegen).

**Fix:** both scans now gate `!ty.is_integer() || is_wide_int_type(ty)` —
fusion/replay is only legal for compare kinds the emit side can actually
fuse (scalar-integer through `emit_int_cmp_impl`). No optimization lost:
i128 compares never flag-fused in reality (the emitter bypass proves it);
they now materialize the boolean normally and the chain emits correctly.

## Validation matrix (fastbuild, this session)

| Check | Result |
|---|---|
| unit | 959/959 |
| correctness | 50/50 |
| regression | 352 + known gcc-14 six (identical to baseline set) |
| sqlite 3.50.4 `-O0` DDL roundtrip | OK |
| sqlite 3.50.4 `-O2` DDL roundtrip | **OK (was SIGSEGV)** |
| zlib-ng 2.3.3 non-compat `-O2` 5MB/20MB/text roundtrip | OK |
| zlib-ng 2.3.3 compat `-O2` levels 1/6/9, 20MB random + text | **OK (was inflateInit2_ crash class)** |
| zlib-ng compat `-O3` / `-Os` | OK |
| fp_die_at_birth (FP-CSE enabled) | MATCH gcc |
| m32 / alias / slot-rmw differential fuzz | (see ledger entry) |

## Procedure post-mortem (session 25 failures → session 26 rules)

Session 25 lost hours to guesswork and a harness crash. Institutionalized:
1. **Auto-save after EVERY validated fix** — commit + regenerate
   `ms178-1.patch` + artifact snapshot in the SAME command chain as the
   validation. No exceptions (this session: snapshot S24).
2. **No bisection on unverified builds.** Session-25's zlib bisection was
   invalid: `make CC=gcc X.o` silently skipped rebuilds (timestamps) and
   arch compiles failed silently (`|| true`). Rule: explicit rc checks,
   never `|| true` on compile steps, rebuild via `rm` + full command.
3. **Right tools over Rätselraten:** `CCC_DUMP_EACH_PASS`+`CCC_DUMP_FUNC`
   (upgraded this session to the compact filtered dumper for ALL pass
   dumps — full-module Debug dumps were unusable on real workloads),
   `LCCC_DBG_FOLD`, targeted backend eprintln probes added/removed per
   diagnosis, gdb watchpoints for first-corrupt-write.
4. **Minimal deterministic repro before touching the workload** (`krep.c`
   was written in one minute and pinned defect 1 exactly).

## Open items (session 27)

1. **i686 acc-cache audit** — complete the cache contract so the
   operand_to_eax hard gate can land there too (currently legacy no-op).
2. **i686 SIB indexed addressing** — emitter implementations for the
   session-25 hooks (`supports_indexed_sym_base`,
   `emit_{load,store}_indexed[_sym]`); est. −300..−500 B boot corpus.
3. Boot-corpus re-measure with regbase folds (kernel tree re-extract).
4. MachInst/mature-boundary audit — the initial (wrong) hypothesis for
   defect 2; the boundary exists and deserves a systematic review even
   though this instance was fusion-scan-side.
