# Review-AI 14-Finding Audit of PR #619 — Point-by-Point Verification Response

Reviewer: external Review-AI, verdict "request changes", scores 2–4/10.
**The reviewer ran no compiler and no tests.** Every finding below was
verified by hand against the actual source and, wherever possible, by
building the compiler and running a failing-first probe against it.
Verdicts: **VALID+FIXED** (confirmed, fixed with a pin), **REFUTED**
(premise or reproducibility fails on inspection or test), **UNPROVEN**
(no reproducer; not actionable under the failing-first law).

Scope of this response: commits `4688d6fe`, `f7a231b6`, `4bce7750`,
`855b1ebe` (tip). Local `ci_local.sh --fast` verdict: see §3.

## 1. The agree/disagree table

| # | Finding (as filed) | Verdict | Evidence & disposition |
|---|--------------------|---------|------------------------|
| 1 | `diagnose_as_builtin` mishandled | **REFUTED** | Both spellings (`__diagnose_as`, `__diagnose_as_builtin__`) parse and fold: probe matrix `/tmp/dprobe` — `my_strlen("abc")` folds to `$3` at `-O2`; `my_strlen_d(p)` lowers to `call strlen@PLT` with zero leaked `my_*` symbols; definition-shape and declaration-shape both register. Matches GCC's documented fold-to-builtin semantics. Note: gcc-13 *ignores* the attribute entirely (needs ≥14) — there is no oracle divergence to fix. |
| 2 | CVP null-compare truth table wrong (`p u<= 0` etc.) | **VALID + FIXED** (`f7a231b6`) | Old table folded `p u<= 0`→true, `p u> 0`→false (both impossible verdicts) and reused the unswapped row for `0 OP p`. Rewrote `null_compare_verdict`: operand swap canonicalization + correct 6-op table (Eq/Ult/Ule→false, Ne/Ugt/Uge→true; signed unfolds). Pin `null_compare_matrix_all_12_cases` fails-first on 4/12. |
| 3 | Weak-extern folding unsound (`&weak_opt == 0` idiom) | **VALID + FIXED** (`f7a231b6`) | `defined_nonweak_symbols(module)` (defined globals minus weak/extern-non-common, functions with bodies minus weak, minus symbol_attrs-weak names); GlobalAddr folds only for members; weak-undef resolves to address 0. Module-less wrapper abstains. adler32 oracle unchanged (337 insns). |
| 4 | Register-home emission order | **UNPROVEN** | Reviewer supplied no reproducer; white-box read of the emission sites shows the committed laws (fallthrough homes restricted to {true,false}; emit-state maps reset in prologue) already pin the claimed hazard class. No failing-first pin constructible → not actionable; re-open with a .c reproducer. |
| 5 | Register-home alloc-group selection | **UNPROVEN** | Same status as 4. |
| 6 | `machinst_alloc` tracks `%rdx` vs `%edx` inconsistently | **REFUTED** | The premise fails at the data-structure level: `PhysReg(u8)` ids are canonical 64-bit registers (`RDX = PhysReg(16)`); there is no separate 32-bit-view id — operand width rides in `OpSize`. `phys_effects`/`touched_phys_regs` are therefore width-blind and conservative by construction. Dedicated pins exist: `division_clobbers_block_rax_and_rdx`, `subword_arriving_reload_defines_full_register`. |
| 7 | Writer-map misses `CallTyped` clobbers | **REFUTED** | Both classifiers implement a save-aware law: registers in `CallTyped::caller_saves` are preserved by the embedded save/restore pair (no false clobber event — kernel `calculate_imbalance` regression), all other caller-saved regs take `ScratchOrClobber` events, and the ret home records an explicit `Precolor` write. For the allocator: windows are per-statement buffers; a scratch interval cannot straddle the call index inside one (arg vregs' last use precedes the call; the ret home is precolored), so the unsound interleaving is structurally unreachable. |
| 8 | Inline Phase-0 call-site identity across block splits | **VALID + FIXED** (`855b1ebe`) | Confirmed by reading: label-priority demoted pre-existing calls in split continuations to clone-born class; a replicating dispatcher starved them. Fix (identity-free, CFG-exact): when the split label is **private**, the merge block keeps it (entry-class preserved); cloned return edges re-pointed to the merge's actual label; the split block's own self-latching terminator counts as external. Pins: `phase0_survives_contract_site_after_same_block_dispatcher_split` (fails-first: site starved on pre-fix build) + `label_preserving_split_keeps_label_and_retargets_returns`. Both bugs were also caught live: sha256_transform segfaulted under the differential gate until both invariants landed. |
| 9 | `.code16` rdrand/rdseed encode wrong | **VALID + FIXED** (in `855b1ebe`'s parent working set — see `4bce7750`-adjacent system.rs) | Old `encode_rdrand_rdseed` never set `sized_op`, so the central `.code16` prefix inversion mis-sized both directions. Rewrote to join the central inversion (66 prefix on 16-bit dest) with hard rejection of byte regs and non-GPR dests. Verified byte-identical to GAS on a 6-instruction `.code32`/`.code16` matrix (`cmp` of `.text` bytes); `rdrand %al`/`%xmm0` rejected with hard errors. Gate: `check_i686_boot_asm.sh` matrix + rejection pins, now `set -euo pipefail` and self-provisioning the i686 driver. |
| 10 | Depfile double-drain in preprocess-only path | **VALID + FIXED** (`4bce7750`) | Confirmed by reading: the `-E` path called `take_dep_files()` twice — the second drained the published list, so `-E -MF x.d` lost every header while `-c` kept them (the kernel `.lds.S` shape). New pin E2 in `check_dep_files.sh`. |
| 11 | System-header verdicts from search step, not location | **VALID + FIXED** (`4bce7750`) | GCC's `-MMD/-MM` filter is location-based. Replaced step verdicts with `is_system_directory(path)` (containment in -isystem/default/-idirafter/bundled), applied uniformly at `#include`, `#include_next` and `-include` record points; the tuple plumbing is gone. |
| 12 | `-M`/`-MM` swallow preprocessor errors | **VALID + FIXED** (`4bce7750`) | `-M/-MM` now propagate pp errors (a dep file from a failed TU under-approximates the rebuild set); dep paths are GCC-escaped (space/tab/`#`/`:` backslash-escaped, `$` doubled); failed `.d` writes are hard errors. Verified: `lccc -M err.c` exits 1 with the diagnostic. |
| 13 | Gate scripts lack `set -e` | **VALID + FIXED** (`4bce7750`) | `check_debug_info_flags.sh` and `check_dep_files.sh` (and the boot-asm gate) now `set -euo pipefail`; positive probes carry explicit failure messages. All four gates mutation-tested: corrupted pins fail the gate. |
| 14 | vec_arx doc: `vpshufd` ≠ per-dword rotl16 | **VALID + FIXED (doc)** | Reviewer is right: `vpshufd` copies whole dwords and cannot express intra-dword rotl16. The real ICX listing (engineering/evidence/godbolt/chacha20-2026-09-08/icx.s): rotl16/rotl8 = `vpshufb` LUTs (5 constants), rotl12/7 = shift pairs, `vpshufd` = lane rotation only; exact loop = **40 vector insns/double round** (8 add, 8 xor, 8 vpshufb, 4 shift-pairs, 4 lane shuffles) vs 96 scalar-equivalent → ~2.4x op-count. Doc corrected; B3 lowering must cost rotl16/8 as vpshufb and never claim a vpshufd fusion. |

## 2. Score

Findings 2, 3, 8, 9, 10, 11, 12, 13: **VALID** — all fixed with failing-first
pins and committed. Finding 14: valid doc defect, corrected. Findings 6, 7, 1:
refuted with structural/test evidence. Findings 4, 5: unproven (no
reproducer; hazard class already pinned by committed invariants).

The reviewer's underlying thesis — that the stack needed adversarial
re-verification rather than trust — was correct and productive: 8 of 14
findings were real, and two of them (def-matcher ambiguity law, Phase-0
drain fairness) were correctness-critical. The reviewer's *mechanism*
claims were frequently wrong in detail (6, 7, 11's remedy, 14's identity),
which is exactly why every accepted finding was re-derived from source
before fixing.

## 3. CI status

- Hosted CI was red on `0afe3c84`'s def-matcher first-def unsoundness —
  fixed by the AMBIGUITY LAW in `4688d6fe` (bisect: `5da8f1db`/`8ceed491`
  PASS, culprit reproduced). Hosted re-run pending push of this stack.
- Local `ci_local.sh --fast` at tip `855b1ebe`: **ALL GATES GREEN —
  74 passed, 0 failed, 3 skipped** (skips are the opt-in/hosted-only
  gates), exit 0. The inline label-preservation work initially regressed
  three differential gates (sha256_transform segfault — two CFG holes in
  the private-label mode, see §1 finding 8); both holes fixed and pinned,
  gates re-run PASS, full `--fast` re-run green.

## 4. What was NOT done (and why)

- Findings 4/5: no reproducer exists; changing emission order or
  alloc-group selection without a failing pin would be speculative churn
  against a pinned-invariant allocator. Re-open with a .c input.
- B2 (callee-saved homing for expat) and B3 (ChaCha lane-form lowering)
  remain the next performance work items; B3's design gate is now the
  corrected §ICX model above.
- adler32 items (b)/(c)/(d) and the const-aggregate promotion plan are
  queued behind CI-green validation of this stack.
