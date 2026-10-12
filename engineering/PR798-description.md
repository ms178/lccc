# Suggested PR title

Fix memory and conditional-fold correctness; repair select fusion and retain structural optimizations

# Suggested PR body

## Correctness first

This PR fixes defects already present on main, not just missed optimizations:

- DSE must not compare offsets belonging to different potentially aliasing pointer roots. Checked arithmetic replaces saturating/truncating address proofs.
- Deleting a conditional comparison block must preserve effects and all externally used definitions. The range/phi transformation now checks the complete block and sibling phi inputs.
- Distinct global symbol names may denote the same object through an explicitly declared GNU alias. DSE no longer needs another optimizer to canonicalize those names to be correct.
- Mutable pointer definitions are rejected per value. An order-independent escape graph prevents an unresolved mutable read from incorrectly preserving a “closed” alloca's pending overwrite proof, without disabling elimination on independent objects.

Reproducers and contracts are in `memory_value_forwarding.c`, `adjacent_membership.c`, `dse_global_alias.c` and their Rust tests.

## Repair the reported CI regression

The failed gate is `check_select_from_compare`, not the expected negative-test messages earlier in the log. Disabling only `intbounds` on the reviewed compiler restores the failing benchmark's assembly byte-for-byte to pristine main.

The repair lets flag fusion follow ordinary register copies **only after** a byte extension has established a full 0/1 value. All deleted definitions must remain dead, and the original flags must remain valid. Unknown upper bits, partial writes, high bytes, intervening effects and live copies have explicit refusal tests.

The existing test's kill switch was applied to the generated program and used an unimplemented name. It now disables `setcc_cmov` in the compiler and checks that the unfused shape returns. The positive `cmovbe` assertion is not weakened.

## Retain and strengthen the requested optimizations

- Exact-type store-to-load forwarding, shared early/late memory cleanup, and late DSE remain.
- Pre-inline cleanup has a dedicated disable control and opt-in time/rewrite reporting. Its current compile-time overhead is not yet measured.
- Width-aware bounds keep explicit cast normalization and allow safe typed truncation without inventing upper bits. Switch proofs now have emitter-level tests covering holes, normalization and PIC/absolute/kernel tables.
- Checked vector-address folding remains before allocation, with malformed, overflowing, cyclic and mutable address paths refused.
- Keep corrected compiler selection and symmetric comparison flags. Linker libraries are placed after the source/object while preserving linker-option ordering.
- Keep recoverable checkpoint history as explicitly requested by the owner; publication budgets remain enforced.

## Evidence and adverse results

- The reviewed switch benchmark is 88 instructions with bounds disabled versus 86 with bounds enabled. This is **not** a runtime speedup claim; the older unchanged count predates that change.
- Historical data show a large SipHash improvement, but are not current-head measurements.
- Historical `struct_copy` minimum time regressed 2.3% (low-three: 0.6%); SHA-256 simulated Callgrind instructions regressed approximately 0.028%. nbody was slightly adverse under default linkage and strongly linkage-sensitive.
- The requested all14 performance program is **not complete**. No claim of universal compiler superiority is made.
- The GNU-alias oracle experiment found a disagreement: optimized Clang 23.1 and ICX produced the specified value; GCC 16.2 and ICC did not. Unoptimized GCC/ICC controls agreed. All outcomes are retained; reference agreement is not treated as semantic authority.
- Renamed historical evidence files retain their exact byte hashes. The concise audit distinguishes historical measurements, reviewed-binary diagnostics and unvalidated corrective source.

## Validation and merge status

**Do not merge yet.** Corrective Rust source has not been compiled or executed under the current no-local-build restriction. Offline control/linker tests and formatting do not replace compiler validation. A previously green tree and the old focused matrix do not validate this head.

Required follow-through on the final hosted SHA: Rust contracts, select-fusion positive/negative controls, the expanded 75-arm regression matrix, formatting/clippy and all required GitHub checks. Fresh matched-main Callgrind/native and compile-time measurements are required before new performance claims.

Changes remain on `arena/b9cd0bf1-lccc` in additive commits. No PR split or force-push is requested. This file is a proposed description, not a claim that the hosted PR body or branch has been updated.
