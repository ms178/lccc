# SV-03 — conditional-load sinking: design, red-team record and evidence

Status: implemented, executable-gated on four ISAs, kill-switch documented.
Scope: `src/passes/if_convert.rs`, `src/backend/i686/codegen/peephole.rs`,
four regression scripts, one cross-ISA harness, one census baseline.

## 1. The problem, measured

`§9b/SV-03` names `select_f32`: a `cond ? *p : *q` diamond that stays a
scalar branch diamond. On the host CPU (AVX-512 capable Xeon, family 6
model 106) the pre-change `-O2 -mavx2 -mfma` output for
`tests/simd/scoreboard_kernels.c` contains `ucomiss`/`jbe`, two arm `movss`
instructions and a `movaps` relay. The ordinary `if_convert` diamond
converter **must** refuse it: hoisting both arms into the predecessor would
speculate a `Load`, and a load from an untaken pointer can trap.

Measured baseline (`pick` body, `objdump -d --no-show-raw-insn`):

| state | instructions in `pick` | branches | loads |
|---|---|---|---|
| LCCC before SV-03 (`CCC_NO_COND_LOAD_SINK=1`) | 7 | `je` + `jmp` | 2 `movss` + 1 `movaps` |
| LCCC after SV-03 | 4 | none | 1 `movss` |
| GCC 16.2.0 `-O2` | 4 | none | 1 `movss` |
| Clang 23.1.5 `-O2` | 4 | none | 1 `movss` |

LCCC after SV-03 is `test/cmovne/movss/ret`; GCC and Clang emit
`test/cmove/movss/ret`. Same instruction count, same register pressure, same
memory traffic, no branch. −3 instructions (−42.9 %) versus the unoptimized
LCCC shape. This closes the shape-level gap on this kernel; it is **not** a
claim about whole-compiler performance (§9c.9.7 still records a 2.79×
geomean deficit across 29 SIMD shapes).

## 2. Alternatives considered and rejected (quantified)

| option | pros | cons / measured reason for rejection |
|---|---|---|
| A. Hoist both arm `Load`s into the pred and `Select` the loaded values | simplest, one new `Select` | **unsound**: an untaken `PROT_NONE` pointer would fault. The regression fixture kills this option with a real `SIGSEGV`; GCC/Clang never speculate a guarded dereference here. |
| B. Relax `arm_load_speculation_ok` / add `Load` to `is_side_effect_free` | tiny diff | changes a *global* predicate used by every diamond/triangle conversion; one relaxed guard would license trapping loads in unrelated shapes. Soundness surface far larger than the win. |
| C. Peephole the emitted branch diamond into `cmov` + one load | no IR change | the emitted OFF shape has **two different memory operands** (`(%rdi)` / `(%rsi)`); a text peephole cannot prove the two addresses are interchangeable without redoing address-chain analysis in the backend. Also loses the load-type/segment/volatility proof. |
| D. **Chosen**: speculate the pure ADDRESS chain, emit `Select { ty: Ptr }`, keep exactly one `Load` in the merge | 1 branch removed, 1 load removed, zero extra memory accesses, target-independent IR | requires a full availability/dominance proof, whole-merge Phi accounting and a target-width proof. All three were implemented and fuzzed with unit tests (§4). |

Why D is what a compiler engineer would ship: the *only* speculative work is
address arithmetic, which is pure and cannot trap; the single surviving load
is on the same control-flow path as in the source. There is no new memory
operation, no new SSA value escaping the merge, and the transform composes
with the existing store-sink → diamond-convert fixpoint.

## 3. The transform, invariant by invariant

`plan_cond_load_sinks` / `apply_cond_load_sink` only admit a diamond where:

1. `P` has a `CondBranch` to distinct `T`/`F`; both arms have exactly ONE
   predecessor (`P`) and both branch to the same `M`; `M` has exactly TWO
   predecessors (`T`, `F`). `build_cfg` counts `IndirectBranch`
   `possible_targets` and `Switch` cases, so a computed-goto edge into an arm
   or the merge vetoes the plan.
2. Each arm contains exactly ONE non-volatile `Load`, of a supported scalar
   type (`I8/U8/I16/U16/I32/U32/I64/U64/F32/F64/D32/D64/Ptr`), with matching
   `ty` and `seg_override`. `I128/U128/F128/Void` are refused: their load and
   move paths are multi-register/aggregate.
3. Every other arm instruction is pure (`Cast`, `GetElementPtr`, `BinOp` with
   a non-trapping op, `Select`, `Copy`, `Phi`, `ParamRef`, `GlobalAddr`,
   `Cmp`, `UnaryOp`). `can_trap()` `Div`/`Rem` are refused; speculating a
   division by zero would be a new fault.
4. **Availability is proven, not guessed.** Each operand used by an arm is
   either defined earlier in that same arm (hoisted in order), or its
   defining block is looked up in a `def_block` census and must
   `DominanceChecker::dominates(def_block, P)`. A value defined in `M` sits
   AFTER the arms and cannot dominate `P`. The first revision used "not
   defined in an arm" as the test; that was a genuine use-before-def hole,
   found by red-team and closed. Cross-arm operands are refused (the other
   arm's chain is hoisted later, so its value is not yet defined).
5. **Every Phi in `M` is accounted for.** Once `T`/`F` disappear, `M` has one
   predecessor, so a surviving Phi would name edges that no longer exist.
   The load-result Phi becomes the single `Load` (same destination); every
   other Phi becomes a `Select` in `P` with the ORIGINAL destination and is
   removed from `M` through `retain_insts` (spans/inline ids stay aligned).
   Auxiliary Phi types are restricted to the set the backend's `Select`
   lowering carries without an aggregate path (`I32/U32/I64/U64/F32/F64/Ptr`);
   narrow integers have no `cmov` form and D32/D64 take the FP-blend path.
   The first revision silently left these phis in place — a real invalid-SSA
   hole, found by red-team and closed.
6. Each doomed load result has exactly one use (the merge Phi). A second use
   would dangle after the arms are emptied.
7. The fresh pointer value comes from `IrFunction::sound_next_value_id`,
   which scans every dest and operand, so it cannot collide with a Phi some
   test injects. Dummy spans are pushed through `BasicBlock::push_inst`, so
   `source_spans`/`source_inline_ids` stay in lockstep.

`apply_cond_load_sink` is transactional: it hoists `T`'s chain, then `F`'s,
then the auxiliary `Select`s, then the pointer `Select`, switches `P`'s
terminator to an unconditional `Branch(M)`, replaces the Phi IN PLACE (no
index shift), removes auxiliary Phis, and clears both arms. The pred's
`CondBranch` is gone, so the pass cannot re-fire on its own output.

## 4. Programmatic tests (all pass)

Rust unit tests, `src/passes/if_convert.rs::cond_load_sink_tests` (11):
one-load-shape + `verify_function` IR validity, volatile refusal, two-loads
refusal, second-use refusal, cross-arm-operand refusal, merge-defined-value
(dominance) refusal, auxiliary-Phi→Select conversion + `verify_function`,
mismatched/wide type refusal, trapping-`SDiv` refusal, partial-merge refusal,
kill-switch transactionality (`EnvGuard`).

Rust unit test, `src/backend/i686/codegen/peephole.rs` (1 new + 17 existing):
`select_diamond_exposed_by_late_copy_propagation_still_folds`.

Executable gate `tests/regression/check_cond_load_sink.sh`: `--self-test`
rejects 5 synthetic shape mutations; the main leg compiles/runs LCCC ON,
LCCC OFF, GCC 16.2, Clang 23.1.5 and (when supplied) ICX 2026.2 against a
`PROT_NONE` untaken pointer; asserts all three shapes (`pick`, `pick_idx`,
`pick_chain`) have `cmov` + one load on ON and a branchy two-load diamond on
OFF; asserts `-O0` correctness. Result: all legs PASS, stdout `-13.50`.

Cross-ISA harness `scripts/check_cond_load_sink_cross_arch.py`: freestanding
fixture, LCCC driver → Clang 23 integrated assembler → mold → native/QEMU
execution, ON and OFF, with a Python oracle (`28`). `--self-test` rejects 4
synthetic evidence mutations. Measured:

| ISA | ON insns | OFF insns | ON exit | OFF exit |
|---|---|---|---|---|
| x86_64 | 38 | 39 | 28 | 28 |
| i686 (ILP32) | 76 | 77 | 28 | 28 |
| AArch64 | 50 | 53 | 28 | 28 |
| RISC-V64 | 114 | 116 | 28 | 28 |

All 8 binaries execute to the oracle and every ON build is strictly smaller
than its OFF control. `readelf -h` proves the ELF machine for each.

## 5. Cross-target finding: i686 pointer `Select` and a real backend defect

`IrType::Ptr` is 4 bytes on ILP32 and 8 on LP64. The four-ISA harness caught
that the first cut produced **78 ON vs 77 OFF** instructions on i686 — a
regression. Root cause, found by bisecting with `LCCC_NO_PEEPHOLE=1` and
`LCCC_DEBUG_PEEPHOLE_IN`: the i686 backend lowers `Select` as a branch
diamond with TWO staged moves per arm
(`movl SRC,%eax; movl %eax,%D`). `fold_select_diamond_cmov` exists and is
sound (18 unit tests), and its isolated shape folds to `cmovnel`, but the
pipeline's copy-propagation collapses the staged pair only on the LAST
fixpoint iteration — *after* the in-loop fold has already run. The fixpoint
then converges and exits, leaving the branchy diamond forever.

Fix: a terminal bounded convergence sweep of `fold_select_diamond_cmov` on
the final structured text, with the new regression test above. No cleanup
pass is needed (the fold nops its replaced lines, `build_result` drops them).
Result: i686 ON is now `testl; movl; cmovnel; movl (%esi), %eax` — branchless,
matching GCC/Clang's shape, 76 vs 77 instructions. This fix benefits *all*
i686 selects, not just SV-03. Validated against `i686-asm-diff`:
**6135 passed, 0 failed** (GNU as 2.47.20260726 byte-oracle), plus 40 other
i686/select/peephole/phi gates.

## 6. Best-tool evidence

* **GDB 18.1** (`trace_on.gdb` / `trace_off.gdb`): break at `pick` with
  `p=0x7ffff7fc1000`, `q=0x7ffff7fc0000` (PROT_NONE), `c=1`. ON:
  `AFTER cmovne: rsi=0x7ffff7fc1000`, `AFTER movss: xmm0=3.500000`, no
  SIGSEGV. OFF: `AFTER branch: rip=0x4015b4`, single arm load reads 3.5.
* **objdump 2.47**: ON `test %edx,%edx; cmovne %rdi,%rsi; movss
  (%rsi),%xmm0; ret`; OFF `test/je`, two `movss`, `jmp`, `movaps`, `ret`.
* **readelf 2.47** `--debug-dump=frames`: the `pick` FDE
  `pc=0x4015b0..0x4015bb` is intact; `pick` has no stack frame, so ORC
  metadata is not applicable. ELF class/machine verified for all four ISAs.
* **objtool / pahole**: not applicable to this change and not present in the
  sandbox. `objtool` validates ORC/unwind of Linux *kernel* objects; the
  artifacts here are userspace ELF with DWARF CFI (checked above). `pahole`
  inspects struct layouts from DWARF; the patch introduces no new data
  structure or ABI type — `IrType::Ptr` already exists. Claiming either tool
  as evidence here would be theater, not measurement. Both are honestly
  recorded as unavailable/not-applicable rather than faked.
* **ICX 2026.2**: local provisioning via `scripts/icx_provision.py` failed
  with `TLS/SSL connection has been closed (EOF)` — `apt.repos.intel.com` is
  outside this sandbox's egress allowlist. The gate accepts an `ICX` env
  override; the hosted `icx-oracle` job (which provisions the authenticated
  snapshot from Intel's PPA) now runs this gate with
  `ICX="$LCCC_ICX_BIN"`, so the four-compiler comparison is real on CI, not
  a surrogate. Locally GCC 16.2.0 and Clang 23.1.5 comparisons are real.

## 7. Pre-existing defects found and fixed (not caused by SV-03)

1. `check_asm_goto_phi_edge.sh`: relied on `<stdint.h>/<stdio.h>` for a
   `-m32` leg; failed in the preprocessor on hosts without
   `libc6-dev-i386` (15 missing `bits/*.h` errors). Fixture made header-free;
   `-S` label invariant is now always asserted; link+run is gated on a real
   32-bit capability probe; 3-shape `--self-test` added.
2. `check_scratch_save_elimination.sh` in `scripts/ci_local.sh` was a
   **dangling line** directly under a `gate` definition (no backslash
   continuation): it ran unconditionally on EVERY `ci_local.sh` invocation,
   its exit code 1 was never attributed to a gate, and its stderr polluted
   the summary. Now a properly registered `scratch-save-elimination` gate,
   and its fixture is header-free so `-m32 -S` works. Standalone rc 0.
3. `check_i686_label_ref_safety.sh`: `-m32` link needed 32-bit libgcc. Static
   assembler invariants split out to run unconditionally; link+run on a real
   probe (a compiler link failure when the probe succeeds still fails).
4. `check_computed_goto_labels.sh`: same `-m32` header/link class; header-free
   fixture + capability probe; all label/asm contracts still run.
5. `check_i686_integer_isa_parity.sh`: same class; capability probe around the
   two execution legs; all assembly/veto/macro and GCC-oracle checks still
   run (`oracle: 8 flag sets match 16.2.0`).
6. `i686-asm-diff`: local GNU as 2.47 oracle was missing; symlinked the
   hash-verified cached binutils 2.47 `bin`/`libexec` pair. 6135/0.
7. `.github/ci-lane-plan.json`: registering the new gate required recording
   it in the balanced lane plan (measured 0.30 s, max of 3 local runs,
   floored at 0.25 s), otherwise `check_ci_lane_plan` fails by design.

## 8. Knob contract and census ratchet

`CCC_NO_COND_LOAD_SINK` disables ONLY this transform; the rest of
`if_convert_function` (store sinking, diamond/triangle conversion) is
unaffected. It is a `var().is_ok()` kill switch, consistent with the other
`CCC_NO_*` switches in this pass, and is the mutation control for every
differential gate above. Without it a green A/B gate proves nothing.

`scripts/audit_census.py` ratchets distinct `CCC_*`/`LCCC_*` literals in
`src/`. This change deliberately adds exactly one (`CCC_NO_COND_LOAD_SINK`),
so `knob_literals` moves `564 → 565`. The knob IS documented here, so
`knobs_undocumented` does not grow (it drops `261 → 255` as this and five other
newly documented names are counted). The baseline is re-based in the SAME
commit, with this section as the justification, per the script's contract.
No other ratcheted metric moves: panics, unsafe, ignored tests and the
`fns_over_*` buckets are unchanged.

## 9. Known limits, stated plainly

* Hosted CI must be green before this is called done; local `--fast` is not
  CI-equivalent and is not claimed as such.
* The win is shape-local (`cond ? *p : *q`). §9c.9.7's 29-shape geomean
  deficit is untouched; SV-01's extended acceptance and paired timing remain
  open.
* ICX evidence is hosted-only in this sandbox (documented in §6).
* `pahole`/`objtool` are unavailable/not applicable (documented in §6).
