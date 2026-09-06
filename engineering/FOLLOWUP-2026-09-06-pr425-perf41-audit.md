# Follow-up — PR #425 completeness and PERF-41 audit

Date: 2026-09-06 UTC
Upstream base: `54126902d370b0765577d433b4f860f1f3d0701b`
PR #425 head: `3ae28a4b62727d77c4e05f459ed761bd799d1d7b`
Session branch: `arena/fix-pr425-ci`

## Deliverable policy

The canonical deliverable is one unified binary patch rooted at the current
upstream `main`. It contains:

1. the complete current PR #425 diff, not a reduced reconstruction;
2. the already validated formatting, regalloc-test, and fastbuild-CI repairs;
3. the complete attached PERF-41 work-in-progress change after audit;
4. the CI invocation for the PERF-41 structural oracle; and
5. this audit record.

The attached `ms178-1-raw-file.patch.txt` is a two-mail patch whose first mail
was generated against an older tree and whose final tree also contains the
MachInst work that is already in current `main` through PR #424. Applying it
blindly to current `main` therefore produces conflicts and would duplicate
already-landed changes. I reconstructed both mails on their applicable older
base, compared the resulting file set with PR #425, and retained the current
PR #425 rebase as the authoritative source. The raw patch's six PF-17
regression files and its Rust-2024/toolchain changes are present in the
current PR rebase; older MachInst and prior-backend hunks are already in the
upstream base. No raw optimization was dropped because it was already in
`main`; the final patch is normalized as a single `main..HEAD` diff.

## PERF-41 audit

### IR and target contract

`StrictRecipMulAddF64x4` is a dedicated intrinsic with an explicit eight-operand
contract:

```text
[scalar accumulator, d0, d1, d2, d3, source base, byte offset, private scratch]
```

The pass is gated independently from generic vectorization and requires all of:

- x86-64 target;
- AVX2;
- SSE4.1 for `pinsrd`/`vpinsrd`;
- XMM allocation enabled;
- no forced SSE2 mode.

The pass state is reset on every `run_passes` invocation, preventing a previous
translation unit's ISA state from leaking through the thread-local flag.

### Legality and correctness restrictions

The transform intentionally accepts only a small, auditable shape:

- a zero-based signed-I32 counted loop;
- one scalar F64 reduction and one scalar F64 load;
- one unconditional body/latch;
- one external preheader predecessor;
- no calls, stores, atomics, opaque intrinsics, or extra F64 loads;
- a source address with canonical unit stride;
- a nontrivial I32 denominator DAG that can be cloned lane-by-lane;
- all external values proven available at the preheader;
- no trapping integer operation in the cloned denominator DAG.

The prefix uses `limit - iv > 3` rather than `iv + 3 < limit`, avoiding signed
overflow in the guard. The original loop remains as an exact scalar tail, so
trip counts from zero through three and non-multiples of four are covered by
the original source-order loop.

### Machine-code contract

The x86 lowering:

1. packs four signed I32 denominators;
2. converts them to four F64 values;
3. computes packed reciprocals and products with AVX2;
4. extracts the two upper lanes; and
5. performs four scalar `vaddsd` operations in source order.

When the scalar result is XMM-homed, the accumulator stays in an allocatable
XMM register. Under `CCC_NO_XMM_REGALLOC=1`, the packed product is written once
to the private aligned 32-byte alloca and consumed at offsets 0, 8, 16, and
24. The regalloc classification explicitly keeps the intrinsic result in the
scalar FP domain and prevents the scratch operation from poisoning unrelated
vector accumulator webs.

### Structural and differential checks

`tests/regression/check_strict_computed_recip_codegen.sh` now defaults to the
repository fastbuild binary and is executable. It checks:

- AVX2 + SSE4.1 packed conversion/division/multiply;
- four ordered scalar adds;
- absence of the transformation for baseline targets;
- absence with AVX but no AVX2; and
- absence with AVX2 but SSE4.1 disabled;
- the private-allocation fallback and all four spill offsets.

The check is also wired into `.github/workflows/ci.yml`, in addition to the
ordinary runtime regression corpus.

## Validation record

After applying PERF-41 to the complete PR #425 rebase:

- Rust 1.98.1 fastbuild compilation, opt-level 1, two Cargo jobs: pass;
- `cargo fmt --all -- --check`: pass;
- `cargo test --profile fastbuild --all-targets --locked -j 2`: pass;
- `cargo clippy --all-targets --profile fastbuild --locked -j 2 -- -D warnings`:
  pass;
- PERF-41 structural assembly oracle: pass;
- LCCC and GCC PERF-41 executable outputs: both `0 0 0 1 2`;
- full differential regression corpus: 682 tests, 667 passed, 0 failed,
  13 compare-skips, 2 host skips;
- no whitespace errors in the staged or unstaged diff;
- CI workflow YAML parses successfully.

The runtime and assembly checks establish correctness and intended code shape
for this reproducer. They do not claim a universal workload-speed win or
replace hardware-counter measurements; the attached optimization is isolated,
feature-gated, and fails closed outside its proven shape.

## Remaining work

1. Re-run the complete GitHub-hosted workflow after the new PR is opened; the
   local commands cannot validate action-runner service permissions.
2. Measure PERF-41 on a PMU-capable Raptor Lake host against scalar LCCC, GCC,
   Clang, and ICX with controlled frequency/affinity.
3. Extend the legality model only with a new differential reproducer and
   explicit floating-environment policy; do not broaden the matcher merely to
   increase hit rate.
4. Keep the unified patch anchored to a freshly fetched `upstream/main` before
   publishing if upstream advances.
