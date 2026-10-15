# S104 — bounded native saturating-mask selection

Base remains merged main `2f477ce01749824ccab73ead7ccac90abd77fa08` after
another fetch requested by the user. S103 was saved before this experiment.

## Choice and proof

The previous header-inline-assembly BZHI prototype was rejected for measured
spill/preservation overhead. Raising global inlining budgets would expose
unrelated pressure regressions. Instead, select BZHI from the actual emitted
BMI2 mask idiom, preserving allocator-chosen registers and the original CMP.
This is a target-local instruction selection, not an arithmetic flag workaround
or a claim that shorter assembly must be faster.

The strict window proves AND255, one-bit SHLX mask, subtraction, source mask,
width comparison and unsigned saturation. Both 32/64-bit forms and the
observed relay into the mask register are covered. BZHI already shares the
SHLX BMI2 requirement. Low-byte index wraparound, zero and width boundaries
are preserved; CMP restores all arithmetic flags, including AF. No memory,
interior label/directive, pinned region or statement separator is accepted.
Removed temporary definitions require resolved whole-CFG deadness. Liveness
is refreshed after each rewrite. Selection runs before save/frame cleanup.
Other targets and portable intrinsic headers remain unchanged.

A new whitespace test exposed the shared x86 classifier's space-only leading
indentation. Normalize horizontal spaces/tabs in that single authority,
not in a private directive-prefix allowlist. Replace the old test's deliberate
"all tabs are opaque" assumption with classification parity and explicit
negative tests for tab-led bytes, non-NOP fill, code-mode and unknown directives.
The shared inert-directive law is unchanged. Data does NOT become transparent.

## Red/green and native checks

Initial optimization tests: 3 PASS / 3 FAIL with the no-op implementation.
First implementation: 5 PASS / 1 FAIL (leading-tab classification). The full
suite then exposed the historical tab assumption; the shared normalization and
its stronger directive tests now pass. Final library suite at this checkpoint:
**4855 PASS / 0 FAIL / 9 existing ignores**. Fastbuild/-O1/-j2 passes with
warnings denied; rustfmt applied. Final stable-tree FAST/Clippy remain separate.

Native assembly tests execute both widths and relay forms, original versus
isolated selection versus the full peephole pipeline. **65,536 input pairs ×
12 functions** check results and all six arithmetic flags against independent
unsigned C models. Values include high index bits and low-byte wraparound.
The C fixture additionally checks 65,536 cases against a scalar reference.
Fresh remote executions pass with GCC16.2.0, Clang23.1.0 and the accurately
identified CE ICX2024.0.0 alias. Native latest ICX2026.1.1 is a separate control.

**Performance acceptance pending:** this checkpoint preserves a
correctness-validated candidate, not a measured optimization win or final gate.
The native corpus and complete patched-zstd workload must still be checked.
