# Population-count and CRC optimization contracts

## Scope and design decisions

This work concerns native population-count loop folding, range-aware CRC table
indices, and the backend analyses these transformations consume. It does not
claim optimal machine code for every function, CPU, or workload.

### Separate two different equivalence relations

The bit-clear recurrence matcher may look through conversions preserving the
recurrence's low bits. Its termination proof must instead preserve **zero in
both directions**. Extensions and same-width integer casts preserve zero;
truncations do not without an additional range proof.

For an eight-bit signed recurrence initially equal to -128, the promoted AND
can produce -256 while its truncated next value is zero. Testing the promoted
value takes another iteration. A direct IR regression requires rejection of
that wide predicate, while sibling raw/Eq/Ne predicates on the normalized next
value must still optimize. CFG/SSA, exit PHIs, and debug spans are checked
explicitly after accepted rewrites.

### Facts require unique definitions, not block-order guesses

The CRC proof walks only uniquely defined dependencies. A last-definition map
alone cannot establish that a masked definition reaches a particular use.
Both the narrow-cast canonicalizer and existing mask-distribution caller carry
definition counts into their range queries. Producer/consumer rewrites also
require unique destinations.

The proof walker is iterative and allocation-free, with a 64-node budget.
Cycles and excessive depth return unknown instead of overflowing the stack.
An explicit unsigned truncation itself establishes a range, so the proof need
not inspect unrelated pre-truncation definitions. Rejecting those definitions
unconditionally would lose sound optimizations without improving correctness.

### Native availability is immutable input

`loop_idiom::Capabilities` defaults conservatively and is passed explicitly
from the driver through the pass pipeline. There is no thread-local permission
or test-only permissive production default. x86-64/i686 use the effective
POPCNT permission, AArch64 requires SIMD registers, and RISC-V retains the
original loop. Target permissions are tested independently of code generation.

### Monotone analysis without an arbitrary fixpoint cutoff

Non-negative producer facts transition only from unknown to known. A consumer
worklist propagates those facts to convergence, including reverse-order copy
chains longer than eight scans. Flat linked use lists avoid per-value vector
allocations. Each instruction is initially visited once and subsequently only
when a direct operand becomes known: expected O(V+E) work with hash lookups,
not repeated whole-function scans. Pure cycles do not seed facts.

These semantic facts must still satisfy the backend's representation contract:
upper register bits must be normalized wherever a cast is elided. The analysis
is shared across x86-64/AArch64; it is not a licence to replace arbitrary typed
casts with untyped copies. Invalid constant shift counts do not seed facts.

### Avoid unnecessary i686 accumulator staging

When a narrow producer already has the required full 32-bit extension image,
a cast between distinct callee-saved register homes emits one `movl`, rather
than two moves through EAX. Identical homes emit nothing. Scratch-register,
spill, unknown-image, and incompatible-image paths retain their established
normalization and accumulator-cache behavior. Direct emitter tests assert the
exact zero/one-move cases and required fallback normalization.

### Keep unrelated policy stable

The exploratory block-layout policy remains enabled at `-O1`. Extending the
chain-only policy to that level requires a separate measured tradeoff. Numeric
performance claims without retained measurement context have been removed
from the backlog.

## Reproducible checks

Build compiler/linker using the repository wrapper, which enables warnings as
errors, two build jobs, the fastbuild profile, and swap provisioning:

```sh
scripts/build_lccc_fast.sh
RUSTFLAGS="$(cat target/lccc-rustflags)" \
  cargo test --profile fastbuild --locked --lib -j 2
RUSTFLAGS="$(cat target/lccc-rustflags) -C debug-assertions=yes" \
  cargo test --profile fastbuild --locked --lib -j 2
RUSTFLAGS="$(cat target/lccc-rustflags)" \
  cargo clippy --all-targets --profile fastbuild --locked -j 2 -- -D warnings
```

The dedicated contract checks native/disabled instruction shapes, pass-disable
sensitivity controls, native object assembly/linking, and freestanding execution
on x86-64, i686, AArch64, and RISC-V. `--run` requires the QEMU runners; missing
execution must fail rather than silently count as a pass:

```sh
python3 tests/regression/check_popcount_loop_contract.py --run
python3 scripts/check_ci_gate_parity.py
```

The two primary C fixtures use independent references. Popcount covers every
8-bit and 16-bit value, guarded loops, promoted wide predicates, and unsigned
counter wraparound. CRC checks 4 upper-bit patterns × 256 low bytes × 256
input bytes independently, then also checks multi-byte recurrence state and
nibble indexing. Existing corpus auto-discovery runs both fixtures. The ICX CI
job additionally compares their output against authenticated ICX 2026.2.

```sh
GCC_BIN=/path/to/gcc-16 python3 tests/regression/run_regression.py \
  --filter popcount -j 2
GCC_BIN=/path/to/gcc-16 python3 tests/regression/run_regression.py \
  --filter crc -j 2
```

Use GCC 16.2 and Clang 23.1.3 explicitly for local oracle comparisons. A cached
Rust 1.97.0 toolchain checks the declared compatibility floor; hosted CI also
checks the current stable toolchain. Compiler identities and environment
limitations belong in the PR evidence, not in unqualified performance claims.

## Runtime measurement, separate from code shape

The density witness has deterministic inputs and an independently computed
Python checksum. The script records compiler hashes, versions, explicit flags,
host identity, and all timing samples. Its loop-idiom-disabled LCCC binary is a
causal control. Random, sparse, and dense inputs are reported separately.

```sh
python3 scripts/measure_popcount_loops.py \
  --cc target/fastbuild/lccc --cc /path/to/gcc-16 --cc /path/to/clang \
  --runs 7 --json work/evidence/popcount-timings.json
```

Do not run timing comparisons alongside builds. Process timings include startup
and call overhead; they are not instruction-latency measurements. No result from
this witness proves superiority on unrelated programs or all microarchitectures.
