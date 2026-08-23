# Session 68 — AArch64 GCC torture execution triage and fixes

Date: 2026-08-23
Base: `b7ce7d8cbc57c0bbe0db74e582a70fa36168c76c` (latest `ms178/lccc` main at session start).
Toolchain: Rust/Cargo `1.98.0` via rustup.  AArch64 execution used
`aarch64-linux-gnu-gcc` 14.2, qemu-aarch64 10.0.11, and GNU as 2.44 behind a
local version wrapper because the harness insists on a GAS 2.47 version string.

## Harness command

```bash
python3 scripts/aarch64_execute_suite.py \
  /home/user/src/gcc/gcc/testsuite/gcc.c-torture/execute \
  --lccc target/fastbuild/lccc-arm \
  --assembler /tmp/aarch64-as247 \
  --cross-gcc aarch64-linux-gnu-gcc \
  --qemu qemu-aarch64 \
  --sysroot /usr/aarch64-linux-gnu \
  --flags=-O2 --jobs 2 --timeout 20 --limit 500 \
  --json /tmp/aarch64_tort500_final.json
```

## Result

First 500 sorted GCC `gcc.c-torture/execute` tests after this patch:

```text
SUMMARY FAIL=22 PASS=478
```

This session fixed five AArch64 first-500 failures:

| Test | Was | Root cause | Fix |
| --- | --- | --- | --- |
| `20050121-1.c` | abort | AArch64 `_Complex long double` / F128 second-return and aggregate copy path lost full-precision data | Use FP return-half helpers for F32/F64 second returns and fix ARM memcpy destination preservation used by F128 aggregate copies |
| `20070614-1.c` | abort in previous baseline | Same complex FP return-half family for `_Complex float` | `emit_get/set_return_f{32,64}_second` now routes through `store_float_reg` / `float_operand_to_reg`, handling FP homes, stack homes, and constants |
| `20171008-1.c` | abort | AArch64 compared an IR `I8` value using the whole W register; packed struct bytes c2..c4 polluted `s.c1 != 0` | Subword integer compares now explicitly sign/zero-extend low byte/halfword before `cmp` |
| `20120919-1.c` | abort | Register allocation allowed a half-open handoff where two different operands of the same ALU instruction shared one physical register; codegen reloaded one operand over the other (`s += pi[i]` became `s += s`) | AArch64 integer ALU emission detects same-physical-register distinct operands and reloads one operand from its stack home into scratch |
| `20020413-1.c` | segfault | F128 parameter spill calls `__trunctfdf2`, clobbering x0 before a later GP `ParamRef` assigned to x19 was materialized; x19 then held long-double bits instead of the `int *eval` parameter | ARM GP parameter pre-store now treats x19 as an allocatable callee-saved home (matching `ARM_CALLEE_SAVED`) and saves x0 before F128 FP parameter spill code can clobber it |

## Code changes

- `src/backend/arm/codegen/returns.rs`
  - Use existing FP value-location helpers for second FP return halves.
- `src/backend/arm/codegen/emit.rs`
  - Preserve memcpy destination address across source address resolution.
- `src/backend/arm/codegen/memory.rs`
  - Save direct/indirect memcpy destination address in x11 before source resolution.
- `src/backend/arm/codegen/alu.rs`
  - Repair same-instruction same-register operand hazards by reloading one operand into scratch.
- `src/backend/arm/codegen/prologue.rs`
  - Include x19 in GP parameter pre-store callee-saved set.

## Regression coverage added

- `tests/regression/arm_f128_param_preserves_gp_param.c`
- `tests/regression/arm_subword_compare_masks_packed.c`
- `tests/regression/arm_memcpy_preserves_register_dest.c`
- `tests/regression/arm_alu_overlap_same_reg_operand.c`

## Validation

Focused torture after fixes:

```text
20020413-1.c PASS
20050121-1.c PASS
20070614-1.c PASS
20120919-1.c PASS
20171008-1.c PASS
```

ARM regression filter:

```text
CCC_ARM=target/fastbuild/lccc-arm CROSS_GCC=aarch64-linux-gnu-gcc QEMU=qemu-aarch64 \
  bash tests/regression/run_regression_arm.sh arm
# === ARM Regression: 13 passed, 0 failed, 1 skipped ===
```

Unit subsets:

```text
cargo test --profile fastbuild --lib arm --locked -j 2
# 59 passed; 0 failed; 4 ignored

cargo test --profile fastbuild --lib phi_coalesce --locked -j 2
# 6 passed; 0 failed
```

## Remaining first-500 failures

The remaining 22 failures are now dominated by larger, already-known feature
clusters rather than the AArch64-specific root causes fixed here:

- nested functions / trampolines / non-local gotos: `20000822-1`, `20010209-1`,
  `20010605-1`, `20030501-1`, `20040520-1`, `20061220-1`, `20090219-1`,
  `920415-1`, `920428-2`, `920501-7`, `920612-2`;
- VLA aggregate varargs: `20020412-1`;
- bitfield / scalar-storage-order clusters: `20040709-{1,2,3}`, `20230630-{2,4}`;
- computed-goto label identity/layout: `20041214-1`, `20071210-1`,
  `20071220-{1,2}`, `920302-1`.

These are not papered over. They require dedicated frontend/IR/CFG work and are
listed here so the next session can attack the largest remaining clusters with
proper design rather than test-specific hacks.
