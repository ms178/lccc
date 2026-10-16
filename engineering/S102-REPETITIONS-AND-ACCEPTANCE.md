# S102: repetitions, policy ablation and acceptance boundary

## Isolation from the new directive repairs

Recompiled all45 benchmark sources with the immutable pre-S97 S96 compiler,
using the same current flags: **45/45 complete native ELFs are byte-identical**
to the measured candidate. Thus S97's directive/unknown-exit repairs did not
change generated code in this corpus. This is bounded code identity, NOT
proof that every possible C or assembly input is unaffected. The current
post-fix compiler also freshly passes65/65 semantic cases, empty compiler
stderr, against the independent recorded GCC-O0 outputs.

##31 fresh native pairs, candidate/main

Existing measured binaries reused; randomized paired order,2 excluded warm-ups,
all raw observations retained. Intervals are95% paired bootstrap, not PMU data.

| Case | Median ratio | Interval |
|---|---:|---|
|arith_loop|0.978553|0.945622–1.003771|
|sieve|1.026235|1.020601–1.031404|
|binary_trees|1.006152|0.998863–1.016474|
|fannkuch|1.250053|1.245807–1.253771|
|sqlite_varint|1.021988|1.019877–1.024721|
|linux_find_bit_scaled|0.981823|0.975408–1.011798|
|sha256_transform|1.002959|0.997571–1.010581|
|nbody|0.800300|0.790331–0.821201|

Fannkuch remains unacceptable as a general performance improvement claim.
Sieve/varint retain >2% costs. Four other >1% screen observations are not
confirmed persistent regressions; do not silently discard either experiment.
The nbody improvement persists under this run's explicit arithmetic policy.

## Fannkuch attribution control, NOT an optimization

Nine independent candidate-only pairs changing **only -fwrapv to -fno-wrapv**:
no-wrapv/wrapv median0.793792771, interval[0.783310210,0.805328838]; outputs
match in every run. This demonstrates policy-dependent cost. It does NOT
justify changing user arithmetic semantics, prove the exact responsible pass,
or establish a generic range proof. No flag workaround was applied.

Whole executable disassembly contains14 MOVSLQ instructions with wrapv versus8
without, and8 in main's wrapv binary. Whole-program full-default-N11 Callgrind
Ir: candidate17131993449,main16054326746 (~6.7% more). Instruction count alone
does not explain the entire25% wall-time gap; no unsupported microarchitectural
cause is asserted. Assembly and raw profiles are retained. Concrete follow-up:
bounded-range/nonwrapping proof for permutation/reversal indices, with signed
boundary and overflow counterexamples, before admitting wider address recurrences.

##31 fresh zstd pairs for six worst screen outliers and the best apparent win

| Cell | Candidate/main | Interval |
|---|---:|---|
|ELF L19 compression|0.988004|0.971383–1.014497|
|source L3 compression|1.004378|0.997041–1.039884|
|random two-thread compression|1.000217|0.987927–1.014672|
|RLE long20 compression|1.039133|1.030250–1.048901|
|random long20 decompression|1.000599|0.992183–1.009295|
|source two-thread compression|1.006465|0.977485–1.072747|
|ELF L9 compression (screen's apparent10% win)|0.994775|0.946318–1.026511|

RLE/long20 retains a ~3.9% cost; the other apparent extremes are not confirmed.
Its fresh four-call whole-program Callgrind Ir is84111314(candidate) versus
82008051(main), ~2.56% more; includes shared validation/driver work. No claim
that this fully accounts for timed-region cycles. The apparent10% win is NOT
accepted. Independent GCC CLI decoding still verifies compression outputs.

## Warning qualification and final gate contract

Candidate and main zstd build logs contain zero warnings. GCC16's PIC legacy
v0.1 object emits six maybe-uninitialized warnings. An explicit diagnostic-only
recompile reproduces all6; adding -fno-semantic-interposition eliminates them.
This supports a policy-sensitive diagnostic, not proof of six source bugs or
proof that arbitrary symbol interposition is safe. The measured builds/source
were NOT modified or warnings suppressed; this disclosure prevents an untrue
"all tool output warning-free" claim. Rust warning-denied build and strict
Clippy are distinct gates.

Acceptance remains bounded: corrected compiler/reporting defects and native
controls, not universal zero defects, full hunk-by-hunk closure of24 PRs, or
GCC/zstd superiority. The requested broad performance goal is NOT met.
Final stable-tree FAST must produce a matching external receipt; its actual
result is recorded by the delivery publisher, not guessed in this document.
Full/slow CI is reserved for hosted infrastructure. No external army approval
or physical EPYC/Raptor Lake validation is asserted, and no remote push occurs.
