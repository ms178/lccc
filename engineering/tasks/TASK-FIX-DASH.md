# TASK-FIX-DASH — RISC-V dash FAIL

IDs: LK-05 (was `current_tasks/fix_dash.txt`) · Priority: **P2** ·
Base: f657de55 · dash is a POSIX shell: FAIL on riscv, PASS on
x86/i686/arm. Open since 2026-02-05.

## Objective

Reproduce, classify, and fix. The failure mode is UNKNOWN — it has never
been established whether dash fails to *compile* or fails at *runtime* on
RISC-V. Two sessions deferred this because the sandbox lacks a RISC-V
execution environment; dash sources were fetched and configured in a prior
session's notes.

## Environment prerequisite

qemu-user (`qemu-riscv64`) + a riscv64 sysroot (Debian riscv64 packages
via `dpkg -x`, matching the repo's user-local toolchain pattern). Without
it, record that the task remains blocked — do not guess.

## Files (expected)

RISC-V backend: `src/backend/riscv/codegen/`, `call_abi.rs`,
va_arg handling; the dash build tree.

## Acceptance

- dash `make test` (or the recorded test invocation) passes on
  qemu-riscv64 with the lccc-built binary, matching the x86 behavior.
- Root cause documented (compile error / miscompile / ABI padding /
  runtime env), with a regression test if compiler-side.

## Validation battery

RISC-V unit + regression suites · dash build log archived · cross-check
the same dash binary runs under qemu with a GCC-built riscv64 toolchain as
oracle.

## Do not

- Do not file environment failures as compiler bugs (see DECISIONS.md
  process section: classify {miscompile, unsupported-feature, environment}).
- Do not modify the RISC-V va_arg padding logic without re-running the
  struct{long double} probes (session 90 locked those shapes).
