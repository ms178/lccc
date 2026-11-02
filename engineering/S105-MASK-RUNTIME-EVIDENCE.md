# S105 — real workload evidence and unresolved variance control

This is a preserved intermediate empirical checkpoint, not final FAST acceptance.
S104's compiler sources remain unchanged. Native controls pass25/25 compiler ×
optimization combinations, including actual native ICX2026.1.1. No-BMI2 fallback
passes without SHLX/BZHI. All45 corpus ELFs remain byte-identical to S103.

The initial research script counted symbol/call-name substrings as BZHI opcodes
at O0/O1. Corrected counting is anchored to the disassembled instruction field,
with negative call/symbol controls. Original receipts are retained; corrected
ones explicitly preserve the old substring counts. Actual native BZHI counts:
new O0/O1 zero, O2/O3/Os two; old compiler zero at all five levels. Correctness
results were unaffected. No performance claim rests on the faulty counter.

## Full-size patched-zstd matrix

96 cells, six1MiB inputs, eight profiles,9 paired rounds, all correct, no failures.
Reference is fresh main2f477ce0; exact payload/input hashes and replay are retained.
Ratios are candidate/reference elapsed time (lower is better).

| Reference | Direction | n | GM | AM | Best | Worst |
|---|---|---:|---:|---:|---:|---:|
|main|compress|48|0.995779954|0.996157408|0.869993460|1.056144696|
|main|decompress|48|1.001694711|1.001724594|0.983490942|1.024570889|
|GCC16.2|compress|48|1.613788123|1.639008785|1.045643142|3.149449287|
|GCC16.2|decompress|48|1.406709199|1.418524191|1.034570904|1.713213737|

The large apparent win and worst regressions must NOT be accepted from this screen.
All14 main-relative point regressions >1%, three source-compression hotspots and
two apparent wins received31 additional randomized pairs, targeting400ms per
arm, with two excluded warm-ups. All19 complete, all frame hashes unchanged.
Source/L3 compression: **0.989357176**,95% paired-bootstrap interval
**[0.986812691,0.991170893]**. Random/long20 compression: **1.036771325**,
interval **[1.006655685,1.058527561]**. That regression requires investigation,
not waiver. The apparent13% duplicate/L9 win does not reproduce; its new ratio
is1.049984 with interval[0.992137,1.112986]. Full19-case receipts are retained.

## Instrumentation and attribution

New mask fixture and full1MiB/four-call source/L3 zstd Memcheck:0 errors,0 live
bytes at exit. Source/L3 whole-program Callgrind Ir falls
**289235575 → 284770075**. BIT_addBits exclusive Ir falls
**14286528 → 9821988**; its emitted body falls16→11 instructions without the
rejected inline-assembly spill expansion. FSE_encodeSymbol exclusive Ir stays
42161444: its prologue-style pinned copy is conservatively not rewritten.
The pin is not removed to force an optimization.

Random/long20 four-call profiles: candidate124606288 versus main124606282
instructions (six instructions, not an explanation for a3.7% wall-time gap).
A fresh independent31-pair confirmation with longer samples, disabled ASLR and
a byte-identical baseline control is prepared for both source/L3 and random/long20.
`setarch x86_64 -R` is available here. Its results will be recorded separately;
old ASLR-enabled observations will not be silently pooled or rewritten.

The broad GCC superiority goal is not met. No physical-host PMU claim, exhaustive
PR-hunk approval, or universal zero-warning certificate is implied. Final FAST,
rustfmt and strict Clippy must certify the final stable source tree separately.
