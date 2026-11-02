# SIMD homes, store operands and inventory coverage

## Scope and verdict

The merged review target is main `1366ec87b322755e64d6bc5f2cf8f37db79ca3c4`.
Its tree exactly matches the preceding corrective delivery. This follow-up
changes documentation and tests, not production compiler behavior. A source-only
syntax/token comparison excludes comments and cfg(test) items and checks that
all modified Rust files retain identical production tokens. That comparison
is not Rust type checking, runtime validation or a performance measurement.

The review identifies real documentation drift and a completeness gap. It does
not establish another miscompile or a new optimization opportunity. No numerical
quality rating, compiler superiority or runtime speedup is supported here.

## Home metadata versus results

`vector_home_width` governs backend SIMD home metadata. `vector_result_width`
describes actual results, subject to a separate check of the instruction's
actual destination. The stack-layout pre-scan and copy-coalescing classification
use home metadata only on instructions with a destination; metadata on a store
must not manufacture a result or imply purity.

The named store exceptions are source homes, not result exceptions:

| Opcode | SIMD source home | Actual result |
|---|---:|---|
| VecStoreI64x2 | 16 bytes | none |
| VecStoreI32x4Pair | 16 bytes; memory write is only 8 | none |
| StoreuPs256 / StoreuPd256 | 32 bytes | none |

Other VecStore entries need not expose source-home metadata through this legacy
query. It is not a universal per-operand width descriptor. Comments in the IR,
backend state and slot-assignment code now make these distinctions explicit.
Accumulator tests check both home and result widths; the name-grammar test
states correctly that it checks homes and includes both VecStore exceptions.

## Inventory completeness

Previously, the shared matrix and a second handwritten classification list could
both omit a newly added variant. Counting thirteen entries does not prove
completeness; nor does a name-shape check cover a widthless future name.

`tests/corpus/test_intrinsic_store_inventory.py` independently extracts:

1. VecStore variants from the IntrinsicOp enum;
2. entries from the shared VECTOR_STORES test matrix;
3. entries from the args-write classification.

It requires exact set agreement, uniqueness and an accurate array length. It
fails on unsupported syntax rather than silently dropping variants. Unrelated
tuple variants, including signed scalar FMA, are handled without pretending all
intrinsics are unit variants. The checker is deliberately not a general Rust
parser; a future inventory representation change requires an explicit update.

Its source-only controls remove each of the thirteen matrix entries and each
write-classification entry, add a widthless/underscore variant, duplicate an
entry, add agreeing phantom entries, and exercise comments and unfamiliar
syntax. These are in-memory source mutations, not compiler red/green runs.
Existing hosted Python discovery automatically includes the test. Rust effect
tests consume the one independent matrix and still check actual predicates.

## Legacy 256-bit stores: keep operand roles separate

Lowering classifies StoreuPs256/StoreuPd256 as PtrStore: `dest=None`,
`dest_ptr=Some(destination)`, and exactly one source in `args[0]`. The x86 and
i686 emitters load this source before storing the destination. Their contract is:

- `reads_pointer_arg() == true`: the source can be a read-only memory operand;
- `writes_memory_via_args() == false`: the write is in dest_ptr, not args;
- `Instruction::may_write_memory() == true` for the valid lowered instruction;
- no produced result and no purity claim.

Thus a blanket exclusion of every store from reads_pointer_arg would be wrong
for this operand-role query. Conversely, true does not mean the whole operation
is read-only, does not establish non-aliasing, and does not validate all argument
positions or malformed IR. The explicit-load may_read_memory list is not a
complete summary of memory-folded operand reads. No predicate is broadened.
The old promise that future opcodes cannot silently widen this allowlist was
too strong: SIMD home/result membership also admits opcodes. Such membership
changes still need operand-role review; the VecStore inventory is not a proof
of every intrinsic signature. The documentation now states this limitation.

New IR tests cover both opcodes and check:

- destination writes despite a readable source, including the same SSA pointer;
- rejection of function purity/constness;
- per-parameter read-only proofs: readable source, rejected written destination,
  same-pointer destination, copied alias and overlapping GEP destination;
- preserving source-load folding before the store's own write;
- preventing later forwarding across an intervening destination write;
- the existing position/width classifier accepts args[0] at 32 bytes, not args[1].

The per-parameter proof is not a global no-alias or purity proof. Caller-side
candidate provenance/use checks remain necessary. Existing writes through
dest_ptr are checked separately from the read-side argument allowlist. The
new tests protect those boundaries rather than replacing them with a broader
metadata-driven optimization. Their Rust execution awaits hosted validation.

## Historical evidence and current validation policy

The previous memory/segment audit is explicitly labelled a record of the prior
session. Its retained archive has source deltas, logs, profiles and hashes, but
not the producing compiler executables. The older review-diagnostics payload
also has unresolved producer/compiler provenance. Neither retained bytes nor
successful hash verification is independent reproduction. The historical
279-versus-651 reduction observation is likewise labelled as recorded, not
reproduced by a metadata assertion. Historical payload bytes are preserved.

Per the current instruction, no local compiler build, compiler invocation,
Rust unit-test execution, compile-based regression, Clippy build, fast CI or
performance measurement is performed. Local checks are limited to the source
inventory and its mutations, retained evidence hashes, source syntax/token
comparison, direct formatting, and Python-only CI/script checks.

New tests must be compiled and run by hosted CI on the follow-up's own head.
PR805's checks concern the already-merged preceding tree, not this follow-up.
The follow-up remains explicitly UNGATED until those checks complete; the
earlier fast-pass stamp must not be reused for changed tests.
