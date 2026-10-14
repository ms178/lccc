# PR824: independent follow-up audit and worst15 tuning

Date: 2026-10-14. Working branch: `audit/pr824`.
Upstream base: `3ff36e8592b0d0e26645bf37d6685395b1a6eca5` (PR826).
Imported PR824: `70da244326b9996f614396d79da66d943d4cd9b9`.
The prior PR814 GCC16 TLS-fixture and linker-provisioning fixes are retained.
This report supersedes historical acceptance claims in PR824-REBASE-AUDIT.md;
it does not relabel their earlier failed/mutable CI run as a pass.

## Scope and acceptance boundary

This is a measured improvement and two semantic repairs, not a proof that
all supported programs or architectures are bug-free. Only one of the fifteen
requested workload families improves in the current bounded A/B. Fourteen
remain unchanged. In particular, byte_load_reassembly remains the largest
instruction-count gap. No EPYC7763 native timing gain is claimed.

All compiler builds use fastbuild, optimization level 1, LTO off, exactly two
Cargo jobs, with active 4-GiB swap on the available filesystem. The test and
lint profile is also fastbuild. Full/slow local CI is explicitly not run.
The separately published delivery receipt and exact-tree `ci_local.pass`
stamp determine final Fast-CI acceptance; this pre-gate report is not itself
an acceptance stamp. An exploratory Fast-CI run was stopped before changing
source to repair a subsequently discovered defect; it is not a passing run.

## 1. Packed denominator DAG, without floating-point reassociation

The spectral reduction already packed division/multiplication, but cloned
its integer coefficient expression four times. The new transactional
`StrictPackedDag` translates validated I32 Add/Sub/Mul/And/Or/Xor and constant
0..31 shifts into existing AVX2 I32x8 intrinsics. Constants and dominating
external scalar values are broadcast once in the preheader. Equivalent
commutative integer expressions share a local CSE entry; subtraction and
shift operand order are preserved. Value IDs and immediate shift counts use
disjoint tagged cache keys. Unsupported expressions retain the original
four-scalar-DAG implementation. A rejected plan does not mutate the CFG.

Only low lanes 0..3 reach signed I32-to-F64 conversion. The four unused high
integer lanes perform no memory access or floating-point arithmetic.
`StrictRecipMulAddPackedF64x4` takes `[acc, I32x8, base, byte_offset, scratch]`
and returns scalar F64. The existing division, source operand order,
multiplication and four sequential VADDSD additions remain unchanged.
The existing private-scratch path remains available without scalar XMM homes.

256-bit integer operations are deliberate: existing 128-bit helpers can emit
legacy SSE. Mixing those with the AVX loop would rely on the optional final
VEX-promotion pass, violating the emitter's own transition-cost contract.
The new path is restricted by the existing x86-64 AVX2+SSE4.1 admission gate;
AVX-only, AVX2 without SSE4.1, and baseline targets retain scalar fallback.
An initially over-broad allocator edit incorrectly classified the new scalar
result as a vector producer; it was caught and removed before the first
optimization snapshot. Its retained allocator entries admit vector input
consumption and scalar F64 results, not a SIMD output.

## 2. Missing intrinsic effects and result type

The original strict reciprocal intrinsic, and initially its new sibling,
were absent from the explicit memory-effect and scalar-result tables.
Both read four source doubles and may write their scratch argument. Both now
report these effects; `Instruction::may_write_memory` sees the args-side
write, and the opcode cannot be admitted as an all-arguments-read-only
consumer. Neither is pure or a vector producer. Both report F64 result type.

`strict_reciprocal_effects_and_scalar_result` failed on the original opcode's
missing read classification before the repair, and passes afterwards.
The archived red log is an actual assertion failure, not a compiler-error
substitute. This establishes a metadata-contract defect; it is not presented
as a previously observed end-to-end optimizer miscompile.

## 3. i686 x87 cache precision defect

Extending the independent GCC differential to i686 exposed an observable
pre-existing failure. After a multiply, `fstl slot` rounded the memory copy
but retained the unrounded 80-bit ST(0) value. The adjacent add consumed that
unrounded value, not the F64 IR result. Adjacency and single use prove stack
liveness, not equality of the extended register and its double slot.

Counterexample from the new test: packed_poly, n=3, bias=INT_MIN,
round-to-nearest, finite cancellation inputs. GCC's SSE-double oracle returned
`4.3368087015168744e+82`; LCCC returned `4.3368086152947377e+82`.
The repair uses `fstpl; fldl` when retaining a cached value, so ST(0) actually
contains the rounded F64 result. It does not change the control word, exponent
range or process rounding mode. Existing cache stack-depth/operand-direction
handling is retained. This costs a reload versus the incorrect cache path;
correctness takes priority, and no i686 speed gain is claimed. A future
emitter simplification could avoid redundant cache management entirely.
The broader x87-versus-SSE double-rounding problem is not claimed solved.

## Regression coverage

`tests/regression/check_strict_packed_dag.sh` is wired directly into both
local Fast-CI and hosted CI. The gate-parity census does not trace nested shell
invocations: an initial indirect wiring was rejected and repaired, not waived.
The source-only VecStore inventory now requires the two scalar scratch writers
in addition to (not inside) the vector-store matrix, with missing/extra-opcode
negative tests. The first complete frozen Fast-CI run exposed these integration
failures; it is not an acceptance stamp.
It compiles oracle kernels separately with GCC16.2, and renamed kernels with
LCCC, then links a GCC-built guard-page driver against both objects.

- Three kernels: wrapped polynomial denominator, constant-shift bit algebra,
  and variable-shift scalar-DAG fallback.
- Counts -3..65, eleven signed biases including INT_MIN/INT_MAX and product
  wraparound boundaries; cancellation, signed zero and tiny finite inputs.
- Four rounding modes; compare binary64 bits (NaNs by class) and every C
  floating-point exception flag. No blanket tolerance masks discrepancies.
- Source array ends exactly at an inaccessible page. Nonpositive counts use
  an inaccessible pointer, checking that zero trips do not read it.
- x86-64-v3 and baseline, each with normal allocation, no scalar XMM homes,
  all-stack allocation, and final VEX promotion disabled; plus i686 against
  a GCC `-m32 -msse2 -mfpmath=sse` oracle: **81,972 comparisons pass**.
- Shape assertions prove the new packed path really fired, retains ordered
  scalar adds, contains no horizontal packed add, and keeps unsupported
  variable shifts on the old VEX lane-pack path. ISA fallback assertions
  cover AVX-only and AVX2-without-SSE4.1 as well.
- Rust units cover commutative CSE versus subtraction, signed constants,
  depth cap, valid/invalid constant shift boundaries, variable-shift refusal,
  division/remainder refusal, and intrinsic memory/result classifications.
- AArch64 and RISC-V kernel output was generated with IR/SSA verification and
  assembled by the respective cross assemblers. This is compile/assemble
  coverage, **not** execution or a claim of full cross-target runtime audit.
- The inherited strict reciprocal runtime/stress tests and shape gate passed
  before the expansion. Their variable shift is not evidence that the new
  packed path executes; the dedicated positive fixture supplies that proof.

The floating-point driver uses masked traps. It does not certify trap delivery
order, signaling-NaN payload preservation, or every FP environment extension.

## Worst15 measurements

See `evidence/pr824-session/worst15.md` for all fifteen rows. Both before/after
GCC16.2 comparisons and a fresh saved-PR824-baseline A/B completed with matching
outputs for all fifteen workloads. Registry problem-size macros are preserved,
including the scaled-find-bit and LZ4 selections. Metrics are bounded
whole-program Callgrind Ir, including startup/libraries, not timings, uops,
cycles, or a reconstruction of the user's EPYC paired timing ratios.

Spectral N=128, final A/B: **5,161,234 / 8,258,714 Ir = 0.624944**, or
**37.5056% fewer simulated instructions**. After/GCC16.2 is still **1.900779**:
this does not beat GCC's dynamic instruction count. Three additional paired
runs each produced 5,161,229 / 8,258,709 Ir; their ratio is 0.624943802.
The five-instruction path/startup difference is not interpreted as noise-free
native performance or assigned a confidence interval for speed.

Conditional branch mispredicts are unchanged in the final pair (8,569 each).
D1 misses are 1,939/1,940 and I1 misses 1,423/1,422. Data reads increase
(526,322 versus 362,502), despite lower Ir: packed constant/operand accesses
are not free. A real EPYC rerun must assess latency, instruction throughput,
and load pressure before drawing a timing conclusion. Callgrind's cache
warning describes host-cache discovery; recorded commands explicitly specify
the simulated I1/D1/LL geometry. It is not a compiler build warning.

## Requested compiler oracles

Fresh Compiler Explorer requests completed for all fifteen workloads against
GCC16.2 (`cg162`), Clang23.1 (`cclang2310`), latest ICX (`cicxlatest`), and ICC
Classic2021.10 (`cicc2021100`). Metadata, effective flags, source hashes and
static assembly metrics are archived. All thirty grouped requests passed.
A separate nine-feature Haswell admission canary passed for all four vendors.

Modern compilers use `-O2 -march=haswell -fwrapv -ffp-contract=off`; ICC uses
its recorded CORE-AVX2 dialect and `-fp-model strict -no-fma`. ICC rejected
`-ffp-contract=off` with warning10148 on the first attempt; that attempt was
not accepted or silently counted. These are explicit vendor dialects, not a
claim of identical optimizer semantics or ISA/tuning to EPYC Zen3. ICX is a
moving channel; its returned identity is retained, not renamed a fixed release.
Local LCCC comparison assembly uses x86-64-v3 because it does not accept the
ICC CORE-AVX2 dialect. Static function/TU instruction counts are diagnostic,
not a runtime ranking or a claim that any competitor was beaten.

## Reproduction and evidence

Core commands (run in repository root; all Cargo jobs are two):

```sh
CARGO_PROFILE_FASTBUILD_DEBUG=0 CARGO_INCREMENTAL=1 bash scripts/build_lccc_fast.sh
GCC=gcc-16 bash tests/regression/check_strict_computed_recip_codegen.sh
GCC=gcc-16 bash tests/regression/check_strict_packed_dag.sh
# Use the flags recorded in target/lccc-rustflags for matching Cargo cache.
RUSTFLAGS="$(cat target/lccc-rustflags)" CARGO_PROFILE_FASTBUILD_DEBUG=0 \
  CARGO_INCREMENTAL=1 cargo test --profile fastbuild --lib --locked -j2 strict_
CARGO_PROFILE_FASTBUILD_DEBUG=0 CARGO_INCREMENTAL=1 CI_LOCAL_JOBS=2 \
  bash scripts/ci_local.sh --fast
```

The evidence directory includes complete final measurement manifests (commands,
compiler/binary/source hashes, geometry, output checks and counter hashes),
all15 summary indices, repeated spectral counters, all requested CE manifests,
feature admission, and focused red/green logs. Absolute raw-counter paths name
workspace artifacts, not files presumed to exist in a fresh checkout.
The canonical patch, numbered snapshots, source archive, format-patch series,
bundle and ledger are generated by the existing hardened snapshot script.

## Remaining work / technical debt

1. Re-run the full 45-workload EPYC7763 paired timing suite on the actual VM;
   no native EPYC access or hardware-PMU evidence was available here.
2. Fourteen of the worst15 are unchanged. The byte-load hot loop already has
   a combined 32-bit load; its main gap is scalar loop/bound/IV and missed
   vectorization, not simply another byte-load combine. A biased-bound loop
   rewrite needs a domain/no-wrap proof before widening admission. Broad,
   unproved loop-unroll changes were not shipped to manufacture a benchmark win.
3. Extend genuinely executing AArch64/RISC-V differential coverage and FP trap
   semantics. The present cross-assembler checks do not satisfy that claim.
4. Consolidate opcode effect/result/operand-role schemas to reduce future
   hand-maintained-table omissions; keep scalar result and vector input
   classification distinct. No whole-codebase formal proof is claimed.
5. Simplify the corrected i686 x87 cache or add a proven exact-result policy;
   do not revive unrounded forwarding under ordinary F64 semantics.
6. Full/slow CI belongs to GitHub CI, per the owner instruction. Historical
   PR814 or PR824 gate counts are not substitutes for a current acceptance run.

## Frozen-run integration corrections

The first completed Fast-CI run was 247 pass / 3 fail / 6 skip; rustfmt,
strict Clippy and MSRV passed, but the run was correctly refused acceptance.
Direct gate wiring and the exact scratch-writer inventory corrections above
repair two failures. The third was a linker fixture mixing PATH-selected GAS
2.47 for `ief.s` with GCC's configured system assembler for `ief2.s`; the latter
rejected MOVRS/GOTTPOFF. Both assembly-only fixtures now use the same explicit
`as --64` path. This fixes test-tool consistency, not a linker-code defect.
Controllers were changed only after the failed run exited; a fresh complete
Fast-CI run is required for the new tree. No failure is converted to a skip.

Focused integration rechecks: 11 source-inventory tests pass (including exact
non-store scratch membership), gate parity passes (225 commands / 22 contracts),
and the strict pinned-linker suite passes 303/303, zero warnings/skips, using
BFD2.47.20260726, LLD23.1.4 and mold2.42.1. The final comment-only compiler
rebuild changed the compiler hash but all fifteen generated benchmark binaries
are byte-identical to the archived measurement build; a fresh Callgrind A/B
confirmed that identity and performance result. See the accompanying evidence.
