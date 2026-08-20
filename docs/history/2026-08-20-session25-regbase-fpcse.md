# 2026-08-20 (session 25) — Agent-B v3 audit, regbase folds, FP-CSE enable, sqlite DDL fix

**Base:** `origin/main` @ `e31baa21` (PR #142 — session-23/24 work merged).
**Deliverable:** unified `/home/user/ms178-1.patch`, re-based on latest main.

## Agent-B v3 patch audit verdicts

| Item | Verdict |
|---|---|
| Fixed-point compose+propagate (8 rounds) + safety net in `build_gep_fold_map` | **ADOPTED — superior to my single re-compose.** The safety net (drop entries whose base is still a map key, re-run `retain_ptr_only_uses` per round) is a strict generalisation; deep alias chains my single re-compose missed are resolved. Sound: dropped entries emit their producer instead. |
| `collect_gep_fold_base_links` + RA interval extension for folded const-offset BASES | **ADOPTED with corrections.** Right idea (the base is consumed at the access, RA-invisible). Their x86-64 store-path guards were defense-only; the real gaps were elsewhere (below). |
| Same-register refusal in x86-64 store fast paths | **SUPERSEDED** — with proper interval extension the value and base provably overlap at the access (distinct registers); the refusals were dead code. Not carried. |
| GVN float-CSE still excluded | **DISAGREE — ENABLED** (see root causes below). |
| i686 SIB-indexed spec (symbol-base) | **DEFERRED to session 26.** Plumbing added this session (`supports_indexed_sym_base`, `emit_load_indexed_sym`/`emit_store_indexed_sym` hooks, sym-base branch in `can_indexed_addr_fold`) — emitter implementations are the next step. |
| quadratic_sr / redundant_loads / alias.rs / fuzzers | **Already merged via PR #142** — v3 duplicates them; NOT re-added (no already-merged hunks in the new patch). |
| Docs (session21/23 re-adds) | **REJECTED** — duplicate of main's history docs; would conflict. |

## Bugs found & fixed this session (all validated)

1. **`folded_gep_values` / `gep_base_offset` leaked across functions**
   (`state.rs::reset_for_function` never cleared them). Value IDs restart at 0
   per function, so skipped-GEP IDs from function A fired spurious
   rematerialisations in function B on colliding IDs. The register-base
   relaxation flooded the set, making collisions deterministic (zlib-ng
   gen_bitlen state corruption → SIGSEGV). This also retroactively explains
   the session-23 "unrelated functions perturb each other" fp_die_at_birth
   symptom. **Fix: clear both in `reset_for_function`.**

2. **Default `emit_gep` dropped the base for slot-less register bases.**
   Both the const-offset and value-offset branches keyed on
   `resolve_slot_addr(base)`; a register-only base produced
   `dest = offset + stale scratch`. Hit by `rematerialize_const_addr` once
   register-base folds skipped such GEPs. **Fixes: new `emit_gep_reg_const`
   overrides (x86-64 `leaq off(%base),%dest`, i686 `leal`), x86-64
   `emit_reg_to_acc` override (trait default was a SILENT NO-OP — the same
   trap that bit i686 in session 20), and soundness fallbacks in the default
   `emit_gep` for both branches.**

3. **Silent store/load drops in x86-64 const-offset paths for slot-less
   register bases** (the trailing `else { operand_to_rax }` emitted NO store;
   the FP load `None => return`). **Fixed with register-direct fallbacks +
   `const_offset_fold_reg_base_ok` opt-in hooks (safe-set: x86-64 excludes
   the rdx/r11 staging scratches; i686 restricts to callee-saved
   ebx/esi/edi/ebp, never eax/edx/ecx).** i686 gained register-base
   64-bit-pair load/store paths (previously silent gaps too).

4. **FP-CSE enabled (F32/F64 load CSE in GVN)** — two blockers removed:
   the leak in (1) plus a **call-argument staging hazard**: die-at-birth
   coalescing homed a still-needed FP value in an xmm argument register an
   EARLIER argument's staging overwrote (fp_die_at_birth: `movsd %xmm7,%xmm3`
   staged cn over cd's home before cd was read). **Fix: staging-hazard
   pre-spill in `emit_call_reg_args_impl`** — detects args whose source home
   is an argument register (xmm0..7 / the SysV GPR six) already written by an
   earlier-staged argument and pre-spills them to a transient 16-aligned
   stack area before any staging write (symmetric FP + GPR).

## Validation (fastbuild, this session)

- unit **959/959**, correctness **50/50**, regression **352 + known gcc-14 six**
  (builtin_cpu_supports_raptor, code16_realmode_encoding, fp_domain_crossing,
  has_attribute_in_code, kernel_flags_and_builtins, sqrt_vex_scalar).
- **fp_die_at_birth matches gcc bit-exact with FP-CSE enabled** (-O2).
- **zlib-ng 2.3.3 (default config, SIMD arch files) round-trip OK at -O0 and
  -O2** (5 MB random, compress + decompress + cmp).
- **sqlite 3.50.4 amalgamation -O0: DDL roundtrip OK** (CREATE TABLE + INSERT
  + SELECT) — the session-24 "DDL Funktionsfehler" is fixed (the
  compose/propagate hole AND the cross-function state leak both contributed).

## Open (session 26, documented honestly)

- **sqlite3 -O2 SIGSEGV in `sqlite3KeyInfoFromExprList`** (`mov %al,(%rcx)`,
  store through bad address; reproduces with `CCC_NO_REGBASE_FOLD=1` too →
  NOT the register-base machinery; pre-existing -O2 pipeline/emitter defect).
- **i686 SIB indexed addressing** (emitter implementations for the hooks
  added this session; est. −300..−500 B on the boot corpus).
- Boot-corpus size re-measurement with the regbase folds enabled
  (kernel tree re-extract needed).

## Env gates (A/B tools)

- `CCC_NO_REGBASE_FOLD=1` — force the pre-session-25 alloca-only fold contract.
- `CCC_NO_FOLDED_INDEX_LIVENESS=1` — disable RA interval extension AND
  register-base folds (one contract).
- `CCC_NO_GEP_FOLD=1` — disable all GEP folding.
