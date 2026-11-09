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
unaffected. It is resolved ONCE per process by `passes::cond_load_sink_enabled()`
(`std::env::var_os(..).is_none()` in `src/passes/mod.rs`), matching the
other pipeline-owned knobs, and is the mutation control for every
differential gate above.  Doing the read in `if_convert.rs` instead cost
one extra per-function `environ` scan and tripped the env-read ratchet in
`check_env_test_hygiene.sh` (149 > 148); the ratchet is never raised for a
new switch. Without it a green A/B gate proves nothing.

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


## 10. Hosted-CI red team: six real regressions found and fixed (PR #1153)

The first hosted run of the PR (`42532255016`) was **7 fail / 16 pass / 3
skipping**.  `Test Suite` is only the lane-result aggregator; its assertion
listed `debug, corpus, codegen-a, backend, backend-contracts-b, icx-oracle`,
so there were six independent root causes.  Logs were read through the
authenticated signed-blob endpoint because `gh api .../jobs/<id>/logs`
redirects to a host the sandbox cannot reach.  The earlier "41/41 local
gates" batch did NOT contain three of these gates; a green subset is not a
green CI.

| # | Hosted symptom | Root cause | Fix |
|---|---|---|---|
| 1,2 | `FAIL cond_load_sink_cross (compile:fail)`, `undefined reference to 'main'` in both corpus jobs (1410 passed / 1 failed / 1416 total) | `run_regression.py` globs `tests/regression/*.c` **non-recursively** and compiles each standalone.  The freestanding four-ISA fixture defines `lccc_main` + a linked `_start`, not `main`. | Moved both gate fixtures into `tests/regression/cond_load_sink_cross/` (`host.c`, `cross.c`) — exactly the layout the runner's own diagnostic prescribes — and rewired the shell gate and Python harness.  No fake host `main` was added.  Local `CCC_VALIDATE_SSA=1` corpus: **1329 passed, 0 failed, 3 skipped-compare, 82 skipped-run, 1414 total**. |
| 3 | `FAIL pass-pipeline environment reads grew: 149 > budget 148` and `FAIL environment-mutating tests run behind env_isolate` in `Fast local contracts (shard b)` | The SV-03 kill switch was read inside `if_convert.rs`, and its transactional unit test mutated the process environment without `env_isolate`. | Moved the one-time resolution to `src/passes/mod.rs` (the file the ratchet deliberately excludes) and wrapped the test in `test_support::env_isolate`.  Ratchet back to **148/148**. |
| 4,5,6 | `FAIL sse41 -O2 umin4: missing unsigned native pminud` (also sse41/-O3, avx2/-O2, avx2/-O3) in `codegen-a` and authenticated ICX 2026.2, plus `FAIL: v6_imin_lt not pminsd` in `Assembler and linker contracts` | SV-03 stole `a[i] < b[i] ? a[i] : b[i]` diamonds.  Their guard **already loads both arm addresses unconditionally**, so sinking produced a pointer `Select` plus a THIRD load (`cmovbq` + `movl (%r11)`) where the original `Cmp` + value-`Select` costs two loads and SLP-packs to one `pminud`.  The A/B proof: `CCC_NO_COND_LOAD_SINK=1` restored all six legs; the pass was the cause, not the rebase. | Added a **profitability proof**, not a heuristic: walk the guard's transitive def chain and refuse the sink if any `Load` in it reads an address structurally equal to either arm's load pointer.  Address keys fold `GEP`/`Cast`/`Copy` chains and normalize `base+0` to `base`, so separately materialized `a[i]` GEPs compare equal; opaque/unknown expressions get unique keys (fail closed on equality).  Refusing only declines a rewrite, so it cannot break soundness.  A guard loaded from an unrelated address (`if (*flag) x = *p; else x = *q;`) still sinks. |

Three new mutation-tested unit tests cover the identical-pointer case, the
separately-materialized-GEP case and the unrelated-guard-load positive
control (`14 passed` in the SV-03 test module).  The executable
`check_cond_load_sink.sh` gate now also asserts, ON **and** OFF, that
`umin4` keeps `pminud`, `umax4` keeps `pmaxud` and `v6_imin_lt` keeps
`pminsd`; its `--self-test` grew from 5 to 7 rejected synthetic mutations.
OFF is the non-vacuous control proving no other pass is responsible.

Measured after the fix (local, cached GCC 16.2 / Clang 23.1.5, fastbuild):
`check_lightpixel.py --require-native` 6/6 PASS, `check_bb_slp_v6_codegen.sh`
PASS, `check_env_test_hygiene.sh` PASS, `check_cond_load_sink.sh` PASS,
four-ISA execution proof PASS (8 binaries, oracle 28).  Hosted re-run is the
accepting authority and is polled separately.


## 11. Post-#1153 perfection pass: sound range-gated signedness folding

> Historical measurement record.  Section 12 supersedes the counts, base
> commit, tool versions and CI verdicts below with the final remeasurement on
> `origin/main = 7ad9f30161ddc4bc1ebdea00ddb4ec54f6865498`.  The proof
> invariants in 11.1–11.3 remain the design authority.

This section records a second, independent red team of the uncommitted
follow-up work on the NEW active branch `arena/4058527f-lccc`, based at that
time on `origin/main = 5e9a06a2fab2edd2ee87f9286a2cdf7595acbc66` after a clean
rebase (PR #1160 changed only four preprocessor/test/doc files, with no
overlap).  PR #1153 is merged and untouched.  Every number below was
remeasured on that tree; the earlier base was `6efad731`.

### 11.1 The unsafe shortcut and why it was rejected

An unconditional same-width signedness `Instruction::Cast -> Instruction::Copy`
deletes one reg-reg move per iteration in the SV-03 narrow-load loop
(Callgrind, 100,000 iterations: 1,940,442 / 2,090,441 refs versus the sound
2,040,442 / 2,190,441).  It is WRONG.  The backend deliberately canonicalizes
same-width signedness homes: i686 `cast_normalize_eax`, AArch64 `sxtw` /
`mov w0,w0`, RISC-V `sext.w`, x86-64 `movl`.  `(I8)(U8)255` must be
`0xffffffff`, not `0x000000ff`.  We rebuilt the unsound fold deliberately and
ran the full 22-section / 4-ISA harness: the IR census failed **all 17 guard
Casts** (count 0), while all three host compilers and all four ISAs still
exited with the correct oracle 5.  ABI/backend canonicalization repaired the
upper bits.  **Conclusion: the runtime teeth are not an architecture-safe
unsoundness detector; the per-function IR census is the load-bearing
assertion.**  The harness docstring now says so explicitly.  Conversely,
`CCC_SIMPLIFY_SKIP=cast_sign_range` fails exactly the five folded sections
(count 1) while hosts/cross still exit 5, so the bisection kill switch is
non-vacuous too.

### 11.2 Alternatives quantified

| Strategy | Correctness | Measured cost | Verdict |
|---|---|---|---|
| Keep every same-width signedness Cast | sound | leaves 99,986 refs on the join benchmark | rejected: a proven one-instruction/iteration loss |
| Unconditional Copy | UNSOUND on 4 backends | saves 99,986 refs | rejected by IR census red team |
| Range-gated proof (`cast_sign_range`) | sound | ON 2,640,446 vs OFF 2,740,432 refs (99,986 saved); opaque volatile-Load control 2,040,448 vs 2,040,434 (14 layout refs) | chosen |
| Backend peephole after register allocation | sound in principle | would need 4 ISA-specific patterns plus a proof of non-negativity that RA has already erased | rejected: more code, worse information |
| Runtime guard / speculation | unsound for trapping loads, adds branches | slower | rejected |

GCC 16.2 / Clang 23.1.5 measured 2,541,005 / 2,543,029 refs on the same
synthetic loop: LCCC ON is still ~3.9% slower.  This patch is a measured
shape-local win, **not** a claim of overall compiler superiority.  The
29-shape geomean deficit in §9c.9.7 and SV-01's paired timing remain open.

### 11.3 The proof, invariant by invariant

`sign_bit_provably_clear` fires only for 8/16/32/64-bit single-register
homes.  128-bit split carriers are refused (the earlier `I128 -> U128`
`LShr`-by-64 pack-extract misoptimisation was caught by a unit test).  Sound
producers are: known booleans; `And` with an in-width constant whose sign bit
is clear; `LShr` by `1..bits-1`; `Select` with BOTH arms proved; `Phi` with
EVERY incoming proved; and in-width non-negative constant join arms.  A
top-level constant is deliberately NOT handled: `fold_const_cast` must retype
it (`I32(5) -> U32` is `I64(5)`, `U32 0x80000005 -> I32` is
`I32(-2147483643)`).  The recursive walk is capped at depth 8, so a cyclic
loop Phi terminates by refusal, never stack overflow.  This pass runs BEFORE
`eliminate_phis`, when the IR is strict SSA and every `Value` has exactly one
definition; the `def_counts == Some(1)` guard on `Select`/`Phi` maps is
therefore load-bearing.  Post-phi duplicate `Copy` homes are a backend
representation, not an input.  No allocation in the proof: def maps are flat
`Vec<Option<T>>` indexed by `Value` ID, all lookups O(1).

### 11.4 Programmatic evidence (all PASS unless stated)

* `cargo build --profile fastbuild --locked --bins`: rc 0, **0 warnings**.
  `cargo clippy --lib --profile fastbuild --locked -- -D warnings`: rc 0.
  `cargo fmt --all -- --check`, `git diff --check`: PASS.
* `cargo test --lib --profile fastbuild --locked`: **8637 passed, 0 failed,
  10 ignored, 15.25 s**.  Six focused range/join/constant/kill-switch unit
  tests plus the 128-bit refusal assertion.
* `scripts/check_same_width_cast_range.py`: 33-mutation `--self-test`,
  independent oracle 5, 3 host compilers (`gcc-16 16.2.0`,
  `clang-23 23.1.5`, LCCC), `-O2 -Wall -Wextra -Werror`, 28 IR sections
  (5 folded / 17 guard Casts kept), four ISAs exit 5:
  x86_64 525, i686 924, AArch64 607, RISC-V64 1538 instructions.
  `CCC_VERIFY_IR=abort` + `CCC_VALIDATE_SSA=1` on every host/cross compile.
* `scripts/check_cond_load_sink_best_tools.py`: 12-mutation `--self-test`;
  GDB 18.1 PROT_NONE canary ON/OFF returns `5a/5a`, no SIGSEGV;
  objdump `pick8/pick16` ON 4 / OFF 7 / GCC16 4 / Clang23 4;
  IR join census ON 0 / OFF 1; Callgrind refs above; readelf
  `.text`/`.eh_frame` present with 5 FDE entries (objtool stand-in).
* `scripts/check_cond_load_sink_cross_arch.py`: 4-mutation self-test,
  oracle 51, 8 binaries exit 51: AArch64 117/126, i686 188/192,
  RISC-V64 298/316, x86_64 93/96 instructions.
* `tests/regression/check_cond_load_sink_matrix.py`: 6-mutation self-test,
  82 deterministic shapes, receipt `5d7bed2a19deb1f4`, 12 compile+run
  pairs, pointer-select census 53/53, skipped oracle `icx2026` locally.
* CI static contracts: workflow-shell `ok (14 workflow file(s), 2 rules)`;
  gate parity `570 commands, 41 registered invocation contracts`;
  shard partition `a=87, b=124`; lane plan `backend-contracts=2`,
  `codegen=2`, `frontend=3`, `2987 s`, self-test `19 rules, 30 fixtures`;
  `ci_verify_oracles.sh --require gcc,multilib,valgrind,pyyaml,gdb,qemu,mold`
  PASS with the pinned GCC16/Clang23/GDB18.1/QEMU11.1.3/mold3.0.0.
* Targeted `ci_local.sh --only` gates: **14 selected gates green (0 failed)**, including the newly wired original four-ISA harness and its mutation contracts.

### 11.5 Tool acquisition, stated honestly

The pinned `harness-cache-8` release-tag cache (`d66c5ac881e2`) supplied
Rust 1.99.1, GCC 16.2.0, Clang 23.1.5, GDB 18.1, binutils 2.47, mold 3.0.0,
QEMU 11.1.3 and the Valgrind 3.19.0 Debian package; a 4 GiB `/swapfile` was
active.  PyYAML 6.0.3 was installed as a PyPI wheel because the cached
`python3-yaml` Debian package needs `libyaml-0-2`, which is not in the cache.
`objtool`, `pahole` and `perf` are NOT installed and were NOT faked:
`apt-get download dwarves linux-tools-common` reported "Unable to locate
package"; `git ls-remote https://github.com/acmel/dwarves.git HEAD` succeeded
(`028149ba6ff5...`) but `cmake`, `pkg-config`, `libelf` and `libdw` are absent,
so a source build is impossible here; `pip index versions pahole|objtool|perf`
found no distributions.  objtool is a Linux-kernel ORC validator and pahole a
DWARF/BTF struct-layout tool: this patch changes neither kernel objects nor
struct layouts.  Their applicable equivalents are readelf's ELF/machine/FDE
check, objdump's instruction census and Callgrind's dynamic instruction refs.
ICX 2026.2 is unavailable locally (Intel PPA unreachable, no usable release
binary); the hosted `icx-oracle` lane is wired with `--require-icx` for the
matrix, the 4-ISA cast-range harness and the best-tool evidence.  No local
ICX PASS is claimed.

### 11.6 Design review / known limits

A super-genius compiler engineer would approve the proof's information
boundary: it refuses exactly what it cannot prove, uses the IR's strict-SSA
invariant instead of inventing a second, weaker dataflow analysis, and leaves
the typed constant fold alone.  It would NOT approve claiming "godlike"
efficiency or "beats GCC/Clang/ICX" from these numbers: the synthetic loop is
still slower than GCC/Clang, ICX is hosted-only, and no broad workload sweep
was rerun here.  The 197-gate broad sweep from the previous session had
16 environment/tooling failures unrelated to this patch; hosted CI is the
accepting authority.  No unsafe code, mutex/spinlock, null dereference,
use-after-free, buffer arithmetic, or new environment-read budget was added.
The new harnesses are deterministic and mutation-tested.  Technical debt
closed: stale performance comment, missing Select/Phi/constant-join proof,
missing full-width sign-set and opaque-Load negatives, unwired four-ISA SV-03
harness, missing GDB/QEMU/mold lane probes, and missing invocation contracts.


## 12. Third pass: rebase, ratchet repair, hosted toolchain pinning and fail-fast evidence

### 12.1 Final base and reconstruction

The final branch is one squashed commit on
`origin/main = 7ad9f30161ddc4bc1ebdea00ddb4ec54f6865498`; that main includes
PRs #1161, #1165 and #1166 (vector-store/query-operator fixes, GCC17 torture
CI and last-20-PR red-team repairs).  The rebase preserves every upstream gate,
including `scripts/check_vec_store_contracts.py --self-test` and its normal
run, and adds the PR's four-ISA cross, same-width cast-range, best-tools and
matrix gates.  The lane plan's `predicted_gate_seconds` is recomputed from the
exact `measured_s` sum: shard A `539.32 s`, shard B `558.20 s`, total
`2990 s`.  Static parity reports `571 commands / 42 invocation contracts`,
shard partition `a=88, b=124`, lane self-test `19 rules / 30 fixtures`.

### 12.2 Hosted-red root causes closed without weakening a ratchet

The first hosted run `42617961252` failed four real lanes plus the aggregator:
three hidden `clang-23` alias failures and one audit-census ratchet failure.
The alias failures came from two harnesses that hard-coded `tool("clang-23")`
and one missing `--clang "$CLANG" \\` in the lane-plan step, while the hosted
runner publishes Clang only as
`/home/runner/.cache/lccc/clang-oracle/23.1.5/bin/clang`.  The fix is a shared,
version-verified resolver (`scripts/sv03_toolchain_identity.py`), explicit
`--clang` arguments and 42 parity contracts that pin `CLANG`/`CLANG23` as
hidden invocation channels.  No unpinned PATH fallback is permitted.

The census grew `panic_sites 863 -> 867`, `fns_over_300 233 -> 234` and
`fns_over_500 97 -> 98`.  Rebaselining would have hidden three real code-quality
defects, so they were removed instead:

| Defect | Root cause | Total replacement | Measured result |
|---|---|---|---|
| 4 new panic sites | `CondAddrExpr::key()` used `unreachable!` + `unwrap()`, and the apply phase re-matched a cloned `Instruction` with `unwrap()`/`unreachable!` | exhaustive `match` for all four key shapes; typed `CondLoadCastTail { dest, from_ty, to_ty }` emitted as a Cast with no variant re-match | `panic_sites = 863`, unchanged baseline |
| `simplify_function_with_config` 498 -> 513 | 69-line use-count census inline | `compute_reassociation_use_counts` plus a bounds-checked, saturating `bump_use_count` | `fns_over_500 = 97`, unchanged baseline |
| `simplify_cast` 298 -> 338 | range proof and Cast-chain rewrites inline | independent `cast_sign_range_fold` and `simplify_cast_chain`; each decision is total over its own def maps | `fns_over_300 = 233`, unchanged baseline |

The saturating increment also removes a theoretical `u32` debug-build overflow
abort without changing the profitability verdict: every count above one is
already the refusing case.  New unit tests cover all four address-key shape
classes, typed-tail parsing, saturating use counts, and the extracted range
fold's width/signedness/proof guards.  `git apply`-reconstructed rebase and
all static CI checkers pass.

### 12.3 Local best-tools failure found and fixed

A harness wipe removed the cached toolchains, Valgrind, `/swapfile` and
`target/`.  The `harness-cache-8` tag (`d66c5ac881e2`) restored Rust 1.99.1,
GCC 16.2.0, Clang 23.1.5, GDB 18.1, binutils 2.47, mold 3.0.0 and QEMU
11.1.3; Valgrind 3.19.0 and `libc6-dbg` were installed from the cache's Debian
packages, and a 4 GiB `/swapfile` was reactivated.  The first restored gate
then exposed a real harness defect: a missing Valgrind produced a bare
`assert`, whose empty message became `error = ""`, followed by
`KeyError: 'objdump'` while the script printed unearned `PASS` lines.

`check_cond_load_sink_best_tools.py` now uses one executable-aware
`discover_tool`, names each missing required tool with a `RuntimeError`, and
routes all printing through `report_evidence`.  An incomplete report prints
the cause and all problems to stderr and returns 1; a complete report prints
the same PASS evidence as before.  `--self-test` grew from 12 to 13 rejected
verdict mutations (adding missing Valgrind), and directly proves both report
paths plus `main()`'s missing-tool preflight.  Simulating the old failure
returns rc 1, empty stdout, `verdict=FAIL` and the named cause; no exception
escapes.

### 12.4 Hosted-like no-alias execution proof

A sanitized PATH was constructed with `clang-23 = MISSING`; compilers were
passed as pinned absolute paths and verified as GCC 16.2.0 / Clang 23.1.5.
This reproduces the exact condition that broke hosted shard A, shard B and the
ICX lane.

| Harness | Result | Quantified evidence |
|---|---|---|
| `check_cond_load_sink_cross_arch.py` | PASS | 8 freestanding binaries, four ISAs, oracle 51; AArch64 117/126, i686 188/192, RISC-V64 298/316, x86-64 93/96 instructions ON/OFF |
| `check_same_width_cast_range.py` | PASS | 3 host compilers, 28 IR sections (5 folded / 17 guard Casts kept), four ISAs exit 5; x86-64 525, i686 924, AArch64 607, RISC-V64 1538 instructions |
| `check_cond_load_sink_best_tools.py` | PASS | GDB PROT_NONE ON/OFF `5a/5a` no SIGSEGV; objdump `pick8/pick16` ON 4 / OFF 7 / GCC16 4 / Clang23 4; IR ON 0 / OFF 1; readelf `.text`+`.eh_frame`, 5 FDEs; Callgrind below |
| `check_cond_load_sink_matrix.py` | PASS | 82 deterministic shapes, 12 compile+run pairs, receipt `5d7bed2a19deb1f4`, pointer-select census 53/53, ICX oracle skipped locally |

### 12.5 Host-CPU performance, measured not asserted

Host CPU: `Intel(R) Xeon(R) Processor @ 2.60GHz` (x86-64, SSE4.2/AVX2/FMA
available).  Callgrind counts deterministic instruction references over
100,000 iterations, not wall-clock time, so VM frequency scaling cannot turn
a codegen difference into a false win.  With the restored Valgrind 3.19.0:

| Binary / state | Ir refs | Delta vs LCCC ON |
|---|---:|---:|
| LCCC fold ON | 2,616,224 | — |
| LCCC fold OFF (`cast_sign_range` kill switch) | 2,716,200 | +99,976 (0.99976 / iteration) |
| LCCC volatile control ON | 2,016,216 | — |
| LCCC volatile control OFF | 2,016,202 | 14 layout refs |
| GCC 16.2.0 fold shape | 2,516,773 | LCCC +99,451 (3.952% more refs) |
| Clang 23.1.5 fold shape | 2,518,797 | LCCC +97,427 (3.868% more refs) |

The fold removes one instruction per iteration on the proven join shape; the
opaque volatile-Load control changes by only 14 refs, so the proof refuses
what it cannot prove.  GCC and Clang still emit fewer refs on this synthetic
loop.  ICX 2026.2 is available only in the hosted authenticated lane; no local
ICX result or broad compiler-superiority claim is made.

### 12.6 Final local validation table

| Check | Result |
|---|---|
| `cargo build --profile fastbuild --locked --bins` | PASS, 0 warnings |
| `cargo test --profile fastbuild --locked --lib` | PASS, 8681 passed / 0 failed / 10 ignored |
| `cargo clippy --profile fastbuild --locked --all-targets -- -D warnings` | PASS |
| `cargo fmt --check`, `git diff --check`, Python/shell syntax | PASS |
| `audit_census.py --check` | PASS: 863/94/27/10/233/97/26/565/254 vs baselines 863/94/27/10/233/97/26/565/255 |
| static CI: workflow shell / parity / shard / lane plan + self-test | PASS: 14 files, 571 commands / 42 contracts, a=88 b=124, 2990 s |
| 11 targeted `ci_local.sh --only` groups | PASS: 21 selected gates, 0 failed; 7 optional toolchain gates skipped per invocation |
| four hosted-like no-alias harnesses | PASS, evidence in `/tmp/recovery/evidence/*.json` |

### 12.7 Tool honesty and efficiency verdict

`objtool`, `pahole` and `perf` remain unavailable in this sandbox; the JSON
evidence records `null` for each and performs the applicable user-space
equivalents with `readelf`, `objdump` and Callgrind.  No result from those three
tools is claimed.  GDB 18.1, objdump/readelf 2.47 and Valgrind/Callgrind 3.19
actually ran.  The new helper functions are O(1) per visited definition,
allocate nothing in the range proof, use flat indexed def maps, and fail closed
on unknown IR.  A compiler engineer can approve the information boundary and
the measured one-instruction win, but **cannot honestly call every function
globally optimal**: no exhaustive superoptimizer proof was run, LCCC is still
slower than GCC/Clang on the synthetic loop, and ICX 2026.2 is hosted-only.
This document therefore claims correctness, zero new audit debt, zero warnings
and a measured shape-local improvement, not godlike universal efficiency.

### 12.8 Hosted acceptance

The hosted run is the accepting authority.  This section is written before the
final push; the PR body carries the final run ID and check verdict.  No
production-ready claim is made before every required hosted check is green.
