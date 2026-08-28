# LCCC Follow-up / Kontinuität — Session 89 (2026-08-28, v7 boot-gate round)

**Scope:** close the 32 KiB realmode boot gate on linux-6.18.46 (CachyMod)
after the v7 peephole layer; land the propagation experiment the right way
(census-gated); resolve the P11 anomaly; deliver the unified patch.

**Base:** `origin/main @ 39372911` (PR #272 = merged S01+S02)
**Session commit:** `8ada41c9` (i686 v7 peephole layer)
**Deliverable:** ms178-1.patch — APPLIES-CLEAN on fresh `39372911`,
zero garbage (no env-gated traces; the earlier `CCC_PH_TRACE` /
`CCC_DM_TRACE` probes were removed before commit).

---

## 1. Boot-gate result

| metric | value |
|---|---|
| .text (realmode setup blob) | **23,378 B** |
| gate threshold | 23,384 B |
| `_end` | 31,168 (4 KiB .pecompat headroom intact) |
| verdict | **PASS** |

Progression this round: 23,782 → 23,419 (P15) → 23,746 (ungated const-alias
propagation, REVERTED, +327) → 23,434 (read-permitting canonical scan)
→ 23,407 → 23,393 (P17 v2) → 23,390 → 23,378 (P18). Final state keeps the
1600-byte `_end` headroom below the .pecompat page.

## 2. What landed (all individually measured)

| # | change | Δ.text |
|---|---|---:|
| 1 | P15 16-bit canonical compares (already counted last round) | −15 |
| 2 | P16 const staging (`movl $imm|$sym,%B; movl %B,%C`, census==0 for %B) | −2/site (4 numeric sites fire today) |
| 3 | read-permitting `canonical_16_def` scan (`writes_reg_family`) | −12 (edd/memory/video-vesa) |
| 4 | P17 select-diamond else-hoist (exit-of-pipeline, final text) | −14 (7 sites) |
| 5 | dead-move census fallback (label/ret/window dead-ends) | −92 gross, −83 after the ret-guard |
| 6 | P18 redundant re-zeroing (`xor eax; xor edi; xor eax; …`) | −12 |
| 7 | encoder: `movl $symbol,label` → R_386_32 (pio_ops fn-ptr tables) | keepable, unblocks boot main.c |

## 3. Lessons that cost time — do not re-learn them

1. **Ungated value-text propagation is a trap (third strike).** Rewriting
   readers without proving the staging def dies buys +327 B here (memory
   stores +4 B, live staging +3 B). P16's census-gated form (zero reads
   function-wide) is the only shape that guarantees deletion.
2. **P17 polarity.** The branch already selects the ELSE path; after
   hoisting the else stores, the ORIGINAL condition jumps to the join.
   Emitting the inverted mnemonic swaps the arms' values — caught only
   because a diff audit of cpucheck.s looked wrong. The `invert_jcc` helper
   is now only used to VALIDATE that the site has an invertible condition,
   not to produce the emitted one.
3. **P11 was never firing.** The gate indexed the andl line itself for
   jCC-ness (always false). The corrected consumer scan (`j + 1`) fires
   soundly (flags identical, dominance proof holds — verified with a probe)
   but the single `-g` fire perturbs the shared Phase 3.8 fixpoint: three
   more staging copies survive than the fire reclaims (net +9 in
   validate_cpu). Restored dormant ON PURPOSE; revisit only with a
   post-fire re-fixpoint owned by the pattern.
4. **Return registers are census-invisible.** `%eax`/`%edx` are observed by
   the CALLER at every `ret`; a function-wide zero-read census says nothing
   about them (caught by `return_metadata_limits_edx_liveness_to_wide_
   functions` + `test_reverse_move_elimination`). The fallback refuses both
   unless the function has no `ret` at all. Functions with 64-bit returns
   carry `# lccc-i686-return-uses-edx` — the test fixture needs it.
5. **`classify_body_line` returning `Some(true)` for register-destination
   writes** silently weakened P17's slot-multiset proof (23,393 instead of
   a sound refusal). Reg-dest writes are prep, never hoistable stores.

## 4. Test/verification state

- Unit suite: **1251 passed / 0 failed / 6 ignored** (13 new tests: P15
  narrow/imm8-floor/signed-refusal, P16 numeric/symbol/multi-reader, P17
  polarity + unequal-slot refusal, P18 collapse + consumer safety, census
  fallback + edx-return guard).
- Regression: `scripts/run_regression_suite.sh` **474 PASS / 0 FAIL /
  0 AB-diff** (first full green since before v5 features).
- Boot gate: PASS (numbers above); swap 8 G active throughout; builds with
  the fastbuild preset, `-j2`.

## 5. P11 anomaly — CLOSED

printf.s really held `movl %ebp,%edx; andl $16,%edx; je` ×3 and the census
was right to report 0 ELIGIBLE-AND-FIRING: the gate was broken (§3.3).
After the fix the sites fire soundly but lose net bytes through fixpoint
perturbation (§3.3), so the pattern stays dormant with an honest comment.
The anomaly is explained; no census changes needed.

## 6. Next-session entry points (priority order)

1. **P17 second harvest**: `cmov`-free multi-store diamonds in
   early_serial_console/edd fire today; the refused sites are nested
   diamonds — a recursive body walk would take the remaining ~10 sites.
2. **P0 — i64 mul-accumulate chain fusion** (unchanged from session 88):
   kstrtoull +154, simple_strtoull +80 vs GCC.
3. **P0 — memset → rep stosl** in the boot corpus (GCC emits it; lccc
   inline-loops).
4. **P18 generalisation**: allow exactly one flag-setter between the zeros
   when the second xor is the branch's producer (flag-identical argument
   still holds; needs a flag-consumer liveness check).
5. **P11 revival** only as a standalone post-fixpoint re-pass (§3.3).
6. **CCC_PEEPHOLE_SKIP plumbing for the i686 text pipeline** (the A/B
   harness's skip switch is x86-64-only; the i686 peephole has no per-pass
   kill switch — peephole_ab.py cannot A/B the new patterns yet).
7. P1 items from session 88 (video family, value-width map, vsprintf body)
   unchanged.

## 7. Snapshot ledger

`S03-v7-bootgate` — base `39372911`, head `8ada41c9`, deliverable
ms178-1.patch APPLIES-CLEAN (validated with `git apply --check` on a
pristine base worktree), zero garbage (junk scan: eprintln!/dbg!/TRACE/
env-escape greps all empty on the committed diff).
