# S83 rebase and red-team review (2026-10-30)

The candidate was rebased onto upstream main
`1c2feff17ccf8df00ac76559d9fcf9e65b96f2d7`. The rebase kept the upstream
reducer decomposition, its lower census made possible by that decomposition,
and its LEA/saturating-subtract coverage. The S81/S82 RA, downcount,
transactional-emission, classification, and regression work was then reviewed
against the merged code rather than assumed to survive verbatim.

A fresh fetch before delivery left main at the same hash. Broader CI remains a
GitHub responsibility; no local full-CI or `ci_local.sh --fast` lane was used.

## Upstream decision review

### Strided reducer: keep the decomposition, strengthen refusal

Upstream's seed/epilogue/driver split is a useful structural change. It keeps
value-numbering order and the emitted instruction stream while reducing the
function-size census. That is a justified baseline reduction, not permission
to raise a limit or hide a new long function. The candidate retains that
structure and the upstream X4Compact shape assertions.

The candidate also retains the checks that must surround the decomposition:
prevalidation of both scalar-header Phi destinations, explicit IV presence,
exact emission budgets, and transactional refusal before IR mutation. An
inconsistent multi-IV width is refused in release builds as well as debug
builds. The result is upstream's decomposition plus fail-closed candidate
guards, not a second competing reducer implementation.

Three red-team cases were added or corrected after the rebase:

* an empty preheader with no source spans must remain spanless after successful
  emission (`0 == 0` does not prove that a parallel span array existed);
* each retargeted scalar-header Phi must have exactly one incoming edge for
  the old preheader, matching the IR verifier's duplicate-predecessor rule;
  and
* a duplicated incoming edge refuses the whole transform without changing the
  function.

### Downcount: retain the optimization, remove incidental assumptions

The upstream `0 -> N / +1 / iv < N` to `N -> 0 / -1 / remaining != 0`
transform is sound only for the proven dead, constant-trip recurrence. The
candidate agrees with that narrow optimization and its entry-guard removal.
It does not widen admission to live-out IVs, signed mismatches, non-unit
steps, zero trips, or wrapping recurrences.

The merged implementation matches the guard as a one-instruction slice rather
than assigning semantic significance to instruction index zero. The `iv` and
`next` definitions must each be unique in their small home blocks; the global
validation scan then rejects every other definition and every unapproved use.
This removes two redundant whole-function definition scans without creating a
large def/use side table. Instructions and parallel source spans are cleared
together only after all proofs succeed. Duplicate-definition and span tests
pin these contracts.

### MachInst and RA classification: share contracts, not superficial lists

The allocating and core MachInst last-write classifiers now consume the same
implicit-write iterator. Their ordering and typed-call tie-break behavior stay
intact. This is the correct kind of consolidation: both consumers need the
same register-clobber contract, including call and IPA-preserved homes.
Division/Raw parity, typed-call, allocation, and structural stream/fallback
tests cover the path.

Memory reads, vector results, alias footprints, and SIMD-location requirements
are not interchangeable and were not merged into a semantic superlist. The
vector-location predicate uses central `vector_result_width` metadata plus the
scalar-result `Horizontal*` exception. The latter now covers every horizontal
add/min/max variant; an enum-source-exhaustive regression compares the test set
with `IntrinsicOp`, while stores and scalar square root remain negative
controls. This guard's blast radius is the explicitly opt-in scalar-FP stack
experiment, not the default allocator configuration.

The S81 split-latch diamond remains a narrowly proved RA shape. It does not
admit arbitrary multi-arm regions, old-Phi reads, unsafe source homes, or
caller-saved inheritance across a call.

### Saturating subtract, preprocessor, linker, and test changes

Upstream's scalar and packed saturating-subtract work is retained. The focused
vector gate adds O1/Os and explicit x86-64/x86-64-v3 coverage, and GNU as
checks the baseline ISA instead of an incomplete handwritten opcode denylist.
O1/Os require correct fallback behavior, not forced vectorization.

The preprocessor and linker changes remain orthogonal to vectorization. GCC
macro parity still forbids libc-owned unprefixed limit macros in LP64 and
ILP32. AArch64/RISC-V warn for accepted-but-unimplemented `-z` dynamic-flag
keywords, and `--fatal-warnings` still turns the warning into an error. The
candidate does not claim that these targets emit unsupported DT_FLAGS bits.

## Validation on the corrected build

The final measured `lccc` SHA-256 is
`d9ddce90b56065f23fcfc6a5d3b064ed6e5ed1110e579dfcf5f179a719b4e63d`.

* Pinned Rust O1 `fastbuild`, exactly `-j2`: PASS (5m31s).
* Clippy `--lib --tests`, `-D warnings`: PASS (5m27s); rustfmt and
  `git diff --check`: PASS.
* Consolidated focused Rust selection: **214/214**, with nonzero groups
  `[28 downcount, 11 reducer, 153 RA, 19 MachInst, 2 vector-location,
  1 linker-warning]`.
* Downcount structure/kill-switch/live-out and 25 GCC parity cells: PASS.
* Packed/scalar saturation, vector remainder, O1/Os, SSE2 GAS, and exact-access
  watchpoint gates: PASS.
* Macro parity, LEA narrowing, MachInst stream controls, and IPA interposition:
  PASS.
* Fixed verify-IR subset: 78 files with `CCC_VERIFY_IR=abort`: PASS.
* The split-latch C differential passed 12/12 O1/O2/O3 cells on x86-64, i686,
  AArch64, and RISC-V. x86 references were GCC 16.2; cross references were the
  provisioned GCC 14.2 cross compilers with QEMU 11.1.2.
* Memcheck passed the diamond, strided-reducer, and U16-saturation executables.
* GDB confirms the intended slide latch shape. Objtool returned zero with the
  same three BP/frame-pointer warning classes and offsets as S81, not a new
  warning class. Pahole emitted no type records and is not layout evidence.
* Census: panic 917/917, hot panic 101/101, unsafe 25/25, ignored tests 10/10,
  functions over 500 lines 97/97, undocumented knobs 266/266. Functions over
  300 lines are 233 against baseline 234. No baseline was edited.

## Exact oracle and performance evidence

All native oracle comparisons used local GCC 16.2.0, Clang 23.1.4, and ICX
2026.2.0. The Godbolt script was run offline with explicit local references;
its assembly is static evidence only. For example, LCCC's byte-load `main` has
180 counted instructions against GCC 132, Clang 106, and ICX 72, so no static
"best code" claim is made.

The stronger native protocol used `benchmark_native_batched.py`: direct
measured objects, whole-object identity checks, one neutral GCC 16.2 driver,
fresh child state, randomized order, every repeated stdout checked, CPU
pinning, and 11 retained paired rounds. Flags were
`-O2 -fwrapv -ffp-contract=off -fno-builtin-memcpy -fno-builtin-memset`.
The last two matched flags avoid ICX's private memcpy/memset runtime calls and
were applied to every compiler. The failed neutral-link receipt without them
is retained, not hidden. Ratios below are S83/reference, so below 1 favors S83;
intervals are exploratory bootstrap CIs from the repository helper.

| workload | S83/GCC | S83/Clang | S83/ICX |
| --- | ---: | ---: | ---: |
| byte_load_reassembly | **0.67146** [0.65904, 0.68052] | 1.00644 [0.98266, 1.01320] | **0.56288** [0.54858, 0.57167] |
| zlib_ng_slide_hash | **0.89216** [0.85380, 0.90117] | 1.06986 [1.06395, 1.08867] | 1.03696 [1.03256, 1.04786] |

Thus the current build is faster than GCC on both batched workloads and faster
than ICX on byte-load, but slower than Clang and ICX on slide. Byte-load versus
Clang is inconclusive near parity. These are two workloads, not a corpus win.

The separate 11-round switch process-time screen gave direct paired medians:
S83/GCC 0.99314 [0.97902, 1.00381] (inconclusive), S83/Clang 0.40921
[0.40705, 0.41859], and S83/ICX 0.49585 [0.49365, 0.52086]. It does not isolate
how much of those differences downcount itself causes.

Correctness-gated Callgrind supports the mechanisms but is not a cycle or PMU
model. LCCC/reference simulated Ir was 0.69753/1.09391/0.56853 on byte-load,
0.77049/1.16210/1.11256 on slide, and 1.00000/1.23079/1.24844 on switch for
GCC/Clang/ICX respectively. On switch, Clang and ICX execute fewer simulated
instructions but incur about 1.96 million conditional mispredicts against
about 3.3 thousand for LCCC. Instruction count alone therefore cannot explain
the native switch result.

No additional speculative optimization was folded in from these comparisons.
The post-rebase corrections are correctness and proof hardening; the measured
results validate the retained work without converting remaining gaps into
unproven claims.

## Remaining limits

This is targeted validation, not a candidate-specific GitHub CI result. It
does not establish correctness or performance on the whole corpus, other ISAs,
other CPUs, or arbitrary CFG shapes. Callgrind cache/branch results are
simulated, and native CIs are exploratory. The exact split-latch proof,
opt-in-only vector-location impact, pre-existing objtool warnings, and lack of
pahole type records remain explicit scope boundaries.
