# Intel SDM audit: uniform shifts and REX2 relaxation

This is a **bounded audit tranche, not a completed whole-manual audit**.

## Source acquisition and coverage

The supplied archive was renamed to `intel-sdm-093.zip`, checked for unsafe
members and CRC failures, and extracted. Its SHA-256 is
`02f7a3495aaa785bb491f3098c0d000a28c93c1b55c54a24b825deee3c697a97`.
The combined PDF's SHA-256 is
`a4a62e6a7ba11a76c7753a195b825087306812aac39b930973f9168ee599f321`.
All 5,364 pages were extracted; all 4,106 bookmarks were indexed. The large
source files and full bookmark review ledger are workspace artifacts under
`/home/user/artifacts/instruction-audit/`, not vendored source files.
Extraction and indexing are not semantic review. Eight instruction sections
have partial, explicitly scoped reviews; other bookmark entries are unreviewed.

The shift review concerns only register-only, unmasked 128/256-bit forms:

| PDF page, one-based | Section | Relevant invariant |
| --- | --- | --- |
| 1343–1345 | MOVD/MOVQ | GP MOVD zeroes bits 32–127; MOVQ supplies all 64 count bits; legacy/VEX upper effects differ |
| 1732 onward | PSLL | Uniform count is low **64**, not 32 bits; excessive counts zero lanes |
| 1744 onward | PSRA | Excessive counts sign-fill lanes |
| 1756 onward | PSRL | Excessive counts zero lanes |
| 2498 onward | VPBROADCAST | Low dword/qword replication |
| 2692 onward | VPSLLV | Independent unsigned element counts |
| 2697 onward | VPSRAV | Independent element counts; arithmetic sign fill |
| 2702 onward | VPSRLV | Independent unsigned element counts |

The SDM is internally inconsistent about VEX.128 VPBROADCASTQ: page 2498
explicitly lists that encoding, while page 2506's exception text says VEX.L=0
is invalid. Do not reject the listed encoding on that exception sentence alone;
the native regression executes the 128-bit qword case. This is not authority
for changing any excluded EVEX encoding.

## Compiler change

A GP (V)MOVD/Q followed by a matching VPBROADCASTD/Q and a variable packed
shift can use a uniform packed shift instead. An arbitrary dword broadcast
cannot: its high count dword would be nonzero. Width, register alias, barrier,
mask, and deleted-write liveness checks protect the substitution. In particular,
legacy MOVD/Q must be recognized **before** later VEX conversion. Liveness is
invalidated after a mutation; legacy preservation is not assumed to be VEX
zeroing. The existing vector-copy disable control also disables this rule.

For the real C expression `return a << n` with a 256-bit unsigned vector,
the current compiler emits `vmovd; vpslld; ret`, replacing the previous
five-instruction sequence. This is instruction-count evidence, not a universal
runtime-speedup claim. Real 128-bit C lowering uses other splat shapes and is
not claimed to benefit from this particular three-instruction matcher.

The native integer-model fixture compares original and rewritten assembly
on boundary counts, high-bit counts, and randomized valid/excessive counts,
without C out-of-range or signed-shift assumptions (163,840 vector cases).
The checked-in differential sources compare actual compiler output against
GCC 16.2.0 and Clang 23.1.3 for every valid count and randomized data across
ten operations (448,000 calls). Compile `differential.c` three times with
`PREFIX=lccc_`, `PREFIX=gcc_`, `PREFIX=clang_`, using `-O2 -mavx2`; link the
objects with `differential-driver.c`. These C comparisons intentionally avoid
out-of-range counts, unlike the independent assembly/model regression.

## Linker hardening

The CODE_4_GOTPCRELX matcher must require REX2.M0=0 before changing opcode
8b to 8d. M0=1 selects a different opcode map; a relocation label alone cannot
prove a MOV encoding. GOTTPOFF already checks this bit. The new test exercises
all 256 payload bytes, eight ModRM register fields, and both PIC policies,
checking exact rewritten bytes for accepted map-zero inputs and rejecting
map-one inputs (4,096 decisions). Rejected cases retain the existing GOT path.

APX/REX2 needs its supplementary encoding contracts: this combined SDM does
not contain a REX2 section. The existing subsystem contract
`engineering/subsystems/x86-apx-evex.md` describes M0 and the backend's
encoding policy; the matcher must agree with that contract. ELF relocations
are governed separately by the x86-64 psABI, not inferred from the SDM.
No unsupported relocation is made supported merely because it has a name;
in particular CODE_5_GOTPC32_TLSDESC remains deliberately rejected.

## Still outside this tranche

Other compiler instruction families, assembler encodings, linker transforms,
full ABI/layout review, the remaining SDM sections, and current AMD full-text
cross-checks are not certified here. The current delta also has no new local
ICX comparison or general runtime benchmark claim.

## Recorded local outcomes

The fastbuild compiler build, strict all-target Clippy (`-D warnings`),
6,173 library tests (10 existing ignored tests), and the lccc-ld binary test
passed. All four shift tests and the exhaustive REX2 test ran successfully.
The real-C differential executable reported `448000 compiler differential
calls`. An isolated Rust test assembled from the production matcher/rewriter
and the new exhaustive test failed with exit 101 after removing only the new
M0 guard, and passed with exit 0 with the guard present. This establishes
that the guard's regression test distinguishes the old decision from the fix;
it does not claim native APX execution on this host.
