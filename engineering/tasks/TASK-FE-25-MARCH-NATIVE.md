# TASK-FE-25 — real `-march=native` for Raptor Lake (14700KF)

IDs: FE-25 · Priority: **P1** · Base: f657de55 · Directly serves the
primary performance target hardware.

## Objective

`-march=native` today maps to the hard-coded x86-64-v3 static feature set.
Implement genuine host detection (runtime CPUID in the driver) enabling
AVX2 / BMI1 / BMI2 / F16C / GFNI / VAES / VNNI where present, and expose
the detected feature set to the codegen cost model (IS-21 groundwork).
Keep the exact-allowlist `__builtin_cpu_supports` contract intact.

## Files

`src/driver/` (flag handling, CPUID probe), `src/common/` (feature set
type), `src/backend/x86/codegen/` (CodegenOptions consumers).

## Acceptance

- On the 14700KF: `-march=native -S` shows GFNI/VNNI forms where the cost
  model opts in; `-march=x86-64-v3` output unchanged (bit-identical on the
  kernel corpus when no v4-class feature is usable).
- `-march=native` on a non-AVX512 host never emits AVX-512.
- Clear error (not silent v3 fallback) when CPUID is unavailable.

## Validation battery

`cargo test --lib` (feature-set unit tests with synthetic CPUID leaves) ·
kernel corpus A/B native-vs-v3 on the 14700KF · intrinsics suite ·
bit-identical check for the default flag path.

## Do not

- Do not change the default `-O2` feature contract (baseline remains
  x86-64-v2-compatible unless overridden).
- Do not tie `__builtin_cpu_supports` to the new runtime probe without
  keeping the SIGILL-safe allowlist behavior.
