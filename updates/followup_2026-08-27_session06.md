# LCCC Follow-up / Kontinuität — Session 87 (2026-08-27, v2 round)

**Scope:** v2 codegen optimization round on the rebased main (PR #267 merged
upstream); deliverable regeneration; 32 KiB gate mechanism fully decomposed.

**Base:** `origin/main @ 3db0bbe2` (fast-forward — local tree was content-identical)
**Session commits:** `bc577150`, `6ef51495`
**Deliverable:** ms178-1.patch (v2) — see §4.

---

## 1. The rebase

The user merged the session-86 codegen work upstream (PR #267, squashed as
`6cd4f289`). Local main and origin/main were content-identical (git diff
empty), so the rebase was `git reset --hard origin/main` — zero conflict
surface. **v2 builds on `3db0bbe2`.**

## 2. The pass-gating bug (read this before touching pass lists)

`CCC_DISABLE_PASSES` and the m16 size policy build one comma-separated list;
every pass gate tested it by **substring**. The m16 policy's `postinline`
entry therefore also disabled the `inline` gate: **every -m16 -Os/-Oz
compile ran with the primary inliner OFF** — the corpus where helper
inlining matters most. Session 82's "full pipeline grows cmdline.o 571→713"
measurement was taken with this bug active: the baseline had NO inlining and
the "full pipeline" added inline+postinline+ifconv+gaddrcse+licm TOGETHER.
The conflated measurement is now replaced by per-pass numbers
(CCC_M16_KEEP): ifconv +410, licm +367, gaddrcse +355, postinline +56 —
**all four m16 policy entries are individually correct**; the policy stays.
Fixed by `pass_disabled()` (exact token match). Also un-broke "univsr"
disabling "ivsr".

## 3. The 32 KiB gate is an ALIGNMENT CLIFF (planning-critical)

`.pecompat` requires 4096 alignment (header.S `.p2align 12`, PE-header
semantics; GNU ld produces the same layout on identical objects — verified).
The gate passes iff **pre-pecompat content ends ≤ 24,576**:

```
content_end = 1,166 (bstext..initdata) + .text + 30 (.text32)
gate PASS   ⇔ content_end ≤ 24,576  ⇔ .text ≤ 23,380
```

* Session 82 PASS: content end 24,261 → `_end` ≈ 31,216 (headroom 1,552).
* Session 86 FAIL: content end 25,912 → boundary 28,672 → `_end` 35,280.
* **Now: `.text` 24,240 → content end 25,436 → FAIL by 860 real bytes**
  (the recorded 2,512 = 860 excess + 1,652 alignment waste in the window).

The gate metric for CI: **pre-pecompat content end**, not `_end` — _end
jumps in 4 KiB steps and hides progress inside a step.

## 4. v2 deliverable contents (all individually measured on the corpus)

| # | change | file | Δ.text |
|---|---|---|---:|
| 1 | token-exact pass gating (m16 inliner fix) | passes/mod.rs | −195 |
| 2 | bool-cast resolution for Select range fold | passes/range_check.rs | (part of −175) |
| 3 | phi-diamond branchy range fold | passes/range_check.rs | −175 |
| 4 | reg→reg direct copy emission | i686/codegen/emit.rs | −35 |
| 5 | i64 ALU/compare fast paths (no stack staging) | i686/codegen/i128_ops.rs | −21 |
| 6 | Phase 2g hot-web steal | backend/regalloc.rs | −36 |
| 7 | sext+zext pair fold | i686/codegen/peephole.rs | −14 |
| | **total** | | **−476** |

Validation: cargo 1205/0/6 · regression 467/0/11 AB-diff 0 · i64 torture
5.4M checks (20k rounds × 27 ops × 6 levels vs 32-bit-half references) ·
m16 corpus boot gate script runs clean end-to-end.

## 5. Environment notes (unchanged from session 05)

Same sandbox constraints: 2 vCPU / 4.1 GiB / no swap possible, clang/mold
absent, 32-bit syscalls blocked by seccomp (death-signal harness for runtime
tests), tools in `/home/z/.local-tools` (source env.sh), cargo needs
`CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=gcc RUSTFLAGS=""`.

## 6. Next-session entry points (priority order)

1. **P0 — close the 860 B to the cliff** (`.text` ≤ 23,380): number()/vsprintf
   loop spills (71 store-reload pairs in number() alone), branch-form range
   fold (~40 sites × 2-4 B), scratch-slot store-store elimination.
2. **P0 — born-at-hazard register homes**: a value defined BY divl (rem in
   %edx) must be able to claim %edx — the current `overlaps_inclusive`
   hazard scan excludes it, keeping the whole number() loop slotted.
3. **P1 — wire build_kernel_boot.sh into run_regression_suite.sh**, gate on
   pre-pecompat content end ≤ 24,576.
4. **P1 — value-width map for Copy/phi dests** (byte slots + movb immediates).
5. **P2 — 64-bit multiply-accumulate chains** for the parsers (mull+add/adc).

## 7. Snapshot ledger

`S03-s87-v2-codegen` — base `3db0bbe2`, head `6ef51495`, v2 deliverable
APPLIES-CLEAN. Artifacts: workspace mirror + PolarFS
(/tmp/my-project/lccc-artifacts: tarball, bundle, per-commit series).
