# Generated-code performance backlog

Keep unimplemented opportunities here only when a reproducible workload or
assembly witness exists. Move completed work to its change review with measured
results; do not turn unmeasured expectations into claims.

## Population-count loop idiom

The glibc `bench-pthread-locks` fixture includes the classic loop
`while (x) { x &= x - 1; n++; }`. Current LCCC emits a serial `lea`/`and`/`test`
loop per set bit, while GCC, Clang, and ICX use a fixed-cost population-count
sequence. Investigate a target-independent loop idiom with a target legality
and cost hook, then validate zero and full-width edge cases. Measure runtime and
Callgrind instruction counts on sparse and dense inputs; compare generated
assembly against the configured compiler oracles. Do not force `popcnt` unless
the selected target features permit it.

## CRC32 table index narrowing

The glibc CRC32 witness contains redundant index extension/masking. Trace value
ranges through IR and x86 lowering before adding a peephole: the preferred fix
is range-aware narrowing that can benefit non-CRC code. Validate against the
real workload, an exhaustive byte-index microtest, assembly size, and Callgrind
instruction counts.
