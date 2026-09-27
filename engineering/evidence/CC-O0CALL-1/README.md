# CC-O0CALL-1

## Task

Audit and repair `-O0` call-argument materialization when non-SSA value identities have multiple definitions.

## Hypothesis

The x86 accumulator-address load fast path treated an accumulator-cache hit as a proof that `%rax` held the pointer definition used by the load.  At `-O0`, a numeric value ID can have multiple definitions after phi elimination, so that proof is invalid.  Disabling that SSA-only fast path when register allocation is disabled should force the established stack-home address path.

## Code locations

- `src/backend/x86/codegen/memory.rs`: accumulator-address load selection.
- `src/backend/x86/codegen/emit.rs`: value materialization and accumulator cache.
- `src/backend/stack_layout/`: stack homes used when physical allocation is disabled.
- `tests/regression/o0_call_argument_stack_home.c`: reduced defined-behavior regression shape.

## Baseline commit

`563fb0bcb39488a97a6aa69858458196cf91710c`.

## Candidate commit

`HEAD` (this implementation commit).

## Compiler configuration

`-O0`, x86-64 SysV.  The compiler's Rust build uses the repository `fastbuild` policy when resources permit.

## Target

Container VM, **UNVERIFIED ON TARGET** (the i7-14700KF is not available here).  This is correctness work; no target performance claim is made.

## Benchmark commands

No benchmark applies.  This task changes only the `disable_regalloc` (`-O0`) call boundary.

## Correctness results

- GCC oracle for the new reduced regression exited 0.
- The requested Rust library test command was started, but the no-swap 4 GiB environment terminated the long Rust compilation before a result was produced.  It must be rerun in CI or a provisioned build environment.

## Performance results

Not applicable.  The optimized SSA path is unchanged.

## Interpretation

The change preserves ABI staging and disables only an SSA-dependent address fast path in the existing non-SSA `-O0` mode, where stack homes are already the authoritative representation.

## Decision

IMPLEMENTED, pending full compiler-suite execution in a provisioned environment.
