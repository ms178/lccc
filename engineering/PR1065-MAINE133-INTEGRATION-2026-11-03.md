# PR1065 main-e133 integration — 2026-11-03

## Scope and honesty boundary

Integrates main `e133233166e702484bcf2abcc0e3b0ba2bb01fc6` into the PR's `891c2266d` state. This is an integration checkpoint, **not completion of the thirteen-family/options or full-kernel acceptance criteria**. The previously hosted full-kernel check failed at `nap_nn_learn_avx2` (2,312-byte frame against 2,048). No validator, frame limit, warning policy, or target compiler was relaxed.

## Semantic resolution

Main adds definition-source spans to tagged record/enum specifiers. Preserve those together with this branch's designated-init bit and scoped enum identities. Struct specifiers now carry eight fields, ending in `designated_init, Span`; union and enum retain main's seven/five fields. Repair the auto-merged seven-field struct and four-field enum patterns as well as the explicit conflicts. Keep both enum identity undo metadata and definition tracking in `TypeContext`.

New differential rows cover designated records with duplicate definitions/inner shadowing, enums in disjoint blocks, restored outer enum definitions and fixed-underlying enum forward completion/shadowing. Extend designated-init differential tests across nested enum scopes and restoration after an inner record shadow.

Kernel preparation used all reported host CPUs, ignoring `LCCC_JOBS`. Match the full build's two-job default and positive-integer validation. Mocked preparation verifies an explicit three-job request, rejects five malformed budgets before side effects, and retains existing provenance, generator and failure-atomicity checks. Mutation replacing the actual `-j"$JOBS"` invocation with `-j2` fails the regression; restored implementation passes.

## Fresh local evidence

All compiler builds use Rust 1.99.1, `fastbuild`, two Cargo jobs, and `-D warnings`. Four GiB swap is active. GCC 16.2.0 and Clang 23.1.4 restored from the digest-verified immutable harness-cache-7 transport. GCC is used only for host tools and explicitly non-kernel differential fixtures, never kernel/boot translation units.

- All compiler/linker target binaries: fastbuild PASS.
- Library tests: **7,833 passed, zero failed, ten existing ignores**.
- Tag redefinitions: **23 cases × three compilers PASS** (LCCC, GCC16.2, Clang23.1.4).
- Designated init: **744 four-target GCC differential checks PASS**.
- Enum tag compatibility: runtime agreement at O0/O2 and incompatible-pointer rejection PASS.
- Kernel provenance and job-budget regression PASS; job-budget mutation rejected.
- Exact Linux v6.18.58 checkout `2926d88bb7fc4608fd942453d929c39e4e5ec006`, 29 package patches, package commit `df9c917272d816361e0da61bb951b5732cb1c662`; LCCC preparation PASS. Git release tag present but GPG signature not independently verified (recorded by the canonical script).
- Canonical boot gate: **31,088 bytes / 1,680 bytes headroom**, 24 ISA-clean objects, GNU ld flat-image equality PASS. Image SHA256 `ea8d15014f3ea5edd988963fb49b2d2dd4880656e631d727ff0de46c286bed7d`.
- Real patched NAP translation unit, unchanged package O3 flags plus strict `-Werror=frame-larger-than=2048`: reproduced failure, forward frame **2,088**, learn frame **2,280** on this merged compiler. These are fresh local values, not the earlier hosted 2,312-byte measurement. Slot census and final IR show numerous surviving 32-byte-aligned min/max/compare temporary homes. Missing raw AVX FP width/operand contracts are under investigation; not fixed by this checkpoint.

## Environment limitations and remaining work

ICX2026.2 authenticated installer and Compiler Explorer requests fail under this sandbox's outbound network allowlist. Do not substitute older tools or present the restored GCC/Clang as ICX evidence. Earlier hosted ICX CI passed on 891c, not this merge. Kernel host tools were restored from the successful hosted prerequisite export on run 41649437116, verifying each part and the complete archive SHA256. Optional development headers are searched after host/sysroot and tools headers, not ahead of Linux's tools/include/linux.

Still required: fix measured NAP frame regression with mutation/runtime evidence, all inventoried warning/option semantics, adversarial optimization/differential gates, actual full link/objtool/QEMU boot, updated remote PR and green hosted checks. No full-kernel success or runtime speedup is claimed here.
