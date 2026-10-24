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
Extraction and indexing are not semantic review. Twenty-two sections
have partial, explicitly scoped reviews; other bookmark entries are unreviewed.

The shift review concerns register-only, unmasked 128/256/512-bit forms. The
128-bit matcher additionally recognizes the exact shuffle/unpack splats below;
256/512-bit coverage is limited to the reviewed broadcast shapes. Word shifts
remain excluded by the executed count-semantics counterexample recorded in the
follow-up journal.

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
runtime-speedup claim. The continued review recognizes 128-bit low-dword
shuffle and low-qword self-unpack splats as well; the measurements below
supersede the initial absence of a 128-bit C improvement.

The native integer-model fixture compares original and rewritten assembly
on boundary counts, high-bit counts, and randomized valid/excessive counts,
without C out-of-range or signed-shift assumptions (163,840 AVX2 cases;
245,760 cases when the host also executes the ZMM extension).
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

## Initial recorded local outcomes

The fastbuild compiler build, strict all-target Clippy (`-D warnings`),
6,173 library tests (10 existing ignored tests), and the lccc-ld binary test
passed. All four shift tests and the exhaustive REX2 test ran successfully.
The real-C differential executable reported `448000 compiler differential
calls`. An isolated Rust test assembled from the production matcher/rewriter
and the new exhaustive test failed with exit 101 after removing only the new
M0 guard, and passed with exit 0 with the guard present. This establishes
that the guard's regression test distinguishes the old decision from the fix;
it does not claim native APX execution on this host.

## Continued review: register roles, byte shifts, and 128-bit splats

Additional scoped manual references (PDF page numbers, not printed pagination):

- PSHUFD: 1715–1716, immediate zero replicates the low dword within a 128-bit lane.
- PUNPCKLQDQ: 1801–1807, self-unpack replicates the low qword.
- UNPCKLPD: 2041–2044, the corresponding bitwise self-unpack has no SIMD FP
  exceptions. It is safe for arbitrary integer bit patterns, including ones
  whose floating-point interpretation is a signaling NaN.
- PSLLDQ: 1730–1731; PSRLDQ: 1754–1755. Legacy forms require XMM registers;
  neither MMX nor GP forms exist. VEX data is register-only; EVEX admits
  memory data. Counts are immediate bytes, not count registers.
- The previously reviewed packed-shift opcode/operand tables were revisited
  for all three data widths, uniform versus per-element count roles, and the
  EVEX-only immediate-plus-memory forms.

### Findings and implementation

1. Mixed-width exemptions let invalid packed-shift data/count registers encode
   as different operands. For example, the old assembler accepts
   `vpslld %ymm1,%ymm2,%ymm3`, but the resulting bytes disassemble as
   `vpslld %xmm1,%ymm2,%ymm3`. Uniform counts must be XMM regardless of data
   width; per-element counts must match the data vector width. The data source
   and destination must also agree. Validate those roles before forced or
   automatic EVEX dispatch, not after the encoder has discarded register names.
2. i686 normally delegates vector-only instructions, but malformed GP data
   prevents delegation and reaches its local VEX encoder. Apply the same role
   validator there, rather than relying on delegation or duplicating the rules.
3. Immediate shifts with memory data require EVEX even for unmasked XMM/YMM
   destinations. Select EVEX from the validated operand shape while preserving
   explicit VEX rejection and the shorter VEX form for legal register data.
4. Legacy PSLLDQ/PSRLDQ used only the destination's register number: `%mm1`
   silently became `%xmm1`. The shared role validator rejects non-XMM names
   in both machine modes, without changing any valid MMX packed-element shift.
5. The compiler's 128-bit scalar-count lowering uses PSHUFD or a qword
   self-unpack rather than VPBROADCAST. Recognize those exact register splats,
   retaining GP move count provenance, mask/barrier/alias restrictions, and
   deleted-write region liveness. A nonzero shuffle selector or distinct
   unpack input is not a uniform scalar splat and is rejected.

The native integer-model test now uses 128-bit shuffle/unpack splats and
256/512-bit broadcasts. Separate structural tests cover legacy unpack spelling,
wrong shuffle selectors, wrong input banks, and mismatched count provenance.
The same existing pass-disable control applies; no new tuning knob is added.

### Reproduction

`check_packed_shift_roles.py /path/to/lccc --report /path/to/report.json`
compares acceptance with the SDM-derived operand-role model and GNU as in both
32-bit and 64-bit modes, then compares every accepted instruction's raw
`.text` bytes with GNU as. The matrix includes immediate, uniform-register,
per-element, memory-count, memory-data, forced-EVEX, forbidden-VEX-memory,
broadcast, mixed-width, GP-substitution and legacy-byte-shift cases. It uses
host binutils (2.40 here), independently of the pinned C compiler oracles.

The retained ten-function C corpus drops from **41 to 36 instructions**:
the three 128-bit dword shifts each fall from 4 to 3; the two 128-bit qword
shifts each fall from 6 to 5. The five 256-bit functions do not change in this
tranche. Counts include returns. Sources, per-function counts, and both
assembly listings are retained here. These are static measurements, not a
claim of a universal runtime improvement or completed vectorizer audit.

The expanded matrix initially used GNU binutils 2.40: **1,808 decisions**, all
**390 accepted cases byte-identical**, and **zero remaining discrepancies**.
The baseline had 372 false acceptances and 44 false rejections. Summary counts
and example inputs are retained in `packed-shift-oracle-summary.json`; full
per-input logs remain workspace artifacts. The script accepts `--gas` and
`--objcopy` to reproduce the same checks with a newer pinned binutils oracle.

Current GNU binutils **2.47.20260726** was restored from the verified source
payload and built locally. The **complete** existing assembler differential
corpora pass: **1,434 x86-64 cases** and **688 i686 cases**, with zero failures.
The expanded 1,808-decision packed-shift matrix also passes against 2.47,
including all 390 exact accepted instruction byte comparisons.

### Vector-oracle fixture corrections and scope

The broader cross-compiler run exposed two fixture defects, not new instruction
semantics to imitate:

- `lane2_v16l` and `lane2_v16d` read index 2 from two-lane vectors. They now
  extract valid index 1, with correspondingly honest function names. All twelve
  constant-extraction functions use a macro that statically checks the *same*
  index it extracts. A deliberate index-2 mutation of the two-lane long case
  fails with `vector lane index out of bounds` in LCCC, GCC 16.2 and Clang 23.1.
- Twenty-four float/double input generators multiplied random floating bit
  patterns by zero, then converted the result to int. A random NaN survives
  multiplication by zero; the conversion is undefined. Construct bounded
  finite values directly from the integer PRNG instead, preserving its draws.

The reference flags now pin FMA enablement and contraction policy explicitly.
The normal GCC gate retains all **11 host-supported runtime levels**, codegen
ceilings and contraction checks, and passes. The gate also accepts repeatable
`--runtime-level 'flags'` arguments for explicit supplemental comparisons; its
assembly-quality and contraction checks still always run.

Clang is not an interchangeable oracle for every GCC ABI/policy configuration:
without AVX, a minimal 32-byte vector identity returns via hidden sret in GCC
and LCCC, but via XMM0/XMM1 in Clang 23.1. With permissive FP contraction,
Clang and GCC/LCCC also choose different allowed fusions for shared products
and loop-carried reductions. Do not treat those ABI/policy differences as
instruction bugs, and do not change the GNU ABI or weaken its default gate to
hide them. The additional Clang run passes **eight common-ABI runtime levels**
with explicit `-ffp-contract=off`, including O0/O1/O2 AVX, O2/Os AVX2,
O3 AVX2+FMA, O2 AVX2 without FMA, and O2 AVX512F. The gate's separate
positive-contraction assembly checks remain enabled. The independent native
integer-shift and original/optimized FP tests are not replaced by this run.

Example supplemental invocation (repeat `--runtime-level` for other levels):

```sh
GCC=/path/to/clang-23 tests/regression/check_vector_ext_codegen.sh \
  --runtime-level '-O2 -mavx2 -ffp-contract=off'
```

The earlier 377-function measurements describe their original pinned corpus,
not a remeasurement of this corrected fixture. Do not mix aggregate counts
across those source identities.

The completed production-source snapshot passes **6,201 library tests**
(10 existing ignored), the linker binary test, strict all-target Clippy,
and the unchanged audit ratchet. The latest main merge reconciles independent
knob additions at 533 literals. No ignored-test, panic, unsafe, large-function
or undocumented-knob budget was increased for this audit tranche.

## Continued review: immediate-memory relocation origins

SDM volume 2A section **2.2.1.6, RIP-Relative Addressing** (combined PDF page
627) defines the origin as the RIP of the **next instruction**. An immediate
following disp32 is therefore part of the distance subtracted in an ELF
PC-relative addend: with imm8 the assembler must use user-addend **minus 5**,
not minus 4. The SDM supplies the address semantics; ELF addend spelling and
symbol arithmetic are independently checked against GNU binutils 2.47.20260726.

Ten EVEX immediate-memory encoder families omitted that adjustment: ordinary
and SAE-capable two-operand immediates, packed shifts, three-source immediates,
compare-to-mask, extraction, rotation, AVX10.2 VMPSADBW, VFPCLASS and VCVTPS2PH.
The common legacy SSE/MMX immediate encoder had the same error, affecting all
27 mnemonics currently dispatched through it. Memory destinations are affected
as well as loads. The new memory-plus-imm8 emission helpers keep the adjustment
and immediate together; they touch only relocations emitted by that operand,
not earlier instructions or absolute-address relocations. The emitted opcodes
and instruction lengths are unchanged.

The i686 address planner also bypassed source-addend normalization for a
broadcast-decorated absolute symbol. `anchor+17{1to16}` could become an undefined
symbol literally named `anchor+17`. It now normalizes pending memory symbols
before GOT/TLS addend policy and before replacing shared EVEX address bytes.
One allocation-free, checked normalization helper is shared with both existing
architecture-specific relocation insertion paths. Overflow retains the source
spelling for the existing diagnostic path; symbol differences retain their
separate resolver contract.

The new ELF differential fixtures cover **258 x86-64 cases** and **45 i686
cases**, including positive/negative user addends, GOTPCREL, absolute controls,
broadcasts and memory destinations. The old binary fails **215 of the 258**
x86-64 cases and **2 of the 45** i686 cases; the fixed binary passes all of them.
The complete corpora now pass **1,692 x86-64 + 733 i686 cases**. The Rust tests
also check prior-relocation isolation, each legacy mnemonic, 16-bit-address
and 32-bit-address i686 broadcast addends, and checked normalization boundaries.
All **6,204 library tests** pass (10 pre-existing ignored tests), as do the
linker binary test, strict all-target Clippy and the unchanged 533-knob census.

`rip-immediate.s` plus `rip-immediate-driver.c` independently exercise a legacy
PSHUFD load, an EVEX packed-shift load and an EVEX extraction store. On this
AVX512F/VL-capable host, GCC 16.2 and Clang 23.1 drivers both pass with the fixed
assembler and GNU assembler objects. With the old object, both detect the wrong
EVEX load/store before the misaligned legacy load faults with SIGSEGV.
`rip-immediate-start.s` provides a libc-free entry point: executables linked
with **lccc-ld** also pass for both fixed and GNU objects and fault for the old
object. These are semantic witnesses, not throughput benchmarks.

Reproduce the positive native witnesses after a fastbuild:

```sh
D=engineering/evidence/instruction-audit-2026-10-24
lccc -c "$D/rip-immediate.s" -o /tmp/rip-memory.o
gcc "$D/rip-immediate-driver.c" /tmp/rip-memory.o -o /tmp/rip-driver
/tmp/rip-driver  # requires AVX512F and AVX512VL; returns 77 otherwise
lccc -c "$D/rip-immediate-start.s" -o /tmp/rip-start.o
lccc-ld -o /tmp/rip-static /tmp/rip-start.o /tmp/rip-memory.o
/tmp/rip-static  # execute only on an AVX512F/VL-capable host
```

This adds a fourteenth **partial** section review, not completion of the
manual, the assembler, or any backend as a whole.

## Continued review: i686 vector relocation ABI

The shared vector encoder's ModR/M/SIB bytes can be correct for i686 while its
relocation types are not. With a 32-bit base register, delegated `@GOT` and
`@GOTOFF` operands became absolute `R_386_32` references; local TLS offsets
could acquire the wrong class or fail translation. The i686 delegation boundary
now reconstructs only the memory relocation from the original displacement,
using the existing i686 symbol, modifier and addend rules. Address bytes and
instruction lengths are preserved, including VEX/EVEX **VSIB gather** fields
and XOP's trailing register byte. Re-encoding these ordinary shared addresses
would unnecessarily risk rejecting vector indices or changing address layouts.
Absolute and 16-bit addressing retain the existing local address planner.

TLSGD/TLSLDM, GOTNTPOFF/INDNTPOFF and TLSDESC protocol relocations cannot be
consumed by these vector instructions; both delegated and local SSE/MMX paths
now reject them rather than lending a TLS relaxation contract to an unrelated
opcode. Valid bare-symbol legacy immediate sources are normalized as absolute
memory without cloning the whole instruction or the operand vector.

This ELF ABI and GNU-syntax review is independent of the SDM's CPU semantics.
Its **430 additional i686 cases** include 360 accepted forms and 70 rejection
controls across six basic modifier classes, positive/negative addends, SIB,
absolute addresses, legacy SSE, VEX, EVEX, XOP, and both gather encodings. Every
case was explicitly checked with GNU binutils 2.47.20260726; no failing oracle
input was trimmed away. The preceding binary fails **292 cases**: 180 wrong
relocation tables, 58 false acceptances and 54 false rejections. The fixed
binary passes all 430. The complete corpora now pass **1,692 x86-64 + 1,163
i686 cases**. All **6,205 library tests** (10 pre-existing ignored tests), the
linker binary test, strict all-target Clippy and the unchanged census pass.
The independent packed-shift matrix still passes 1,808 decisions and 390 exact
accepted-byte comparisons.

`i686-vector-got.s` is a libc-free native 32-bit PIC witness for GOT and GOTOFF
vector accesses. On the recorded Linux host with IA32 execution and AVX512F/VL,
objects produced by the fixed assembler, GNU assembler, GCC 16.2 and Clang 23.1
all execute successfully after GNU `ld -m elf_i386`. The preceding LCCC object
faults with SIGSEGV because it uses absolute addresses where GOT-relative
addresses were requested. This witness does **not** claim i686 linking support
in lccc-ld or general native coverage of TLS models.

```sh
D=engineering/evidence/instruction-audit-2026-10-24
lccc -m32 -c "$D/i686-vector-got.s" -o /tmp/i686-got.o
ld -m elf_i386 /tmp/i686-got.o -o /tmp/i686-got
/tmp/i686-got  # requires Linux IA32 execution and AVX512F/VL
```

## Continued review: legacy SIMD operand shapes and typed shuffles

The continued instruction-table review covers PSHUFW (PDF page 1725),
PALIGNR (1505), ROUNDPS (1875), SHA1RNDS4 (1923), MPSADBW (1421),
GF2P8AFFINEQB (1133), and GF2P8AFFINEINVQB (1130). PSHUFD pages
1715–1717 and MOVDQA pages 1348–1351 additionally establish the register-only
shuffle composition and identity-copy rules. These are scoped reviews, not
certification of those sections' entire arithmetic/exception/EVEX behavior.

A shared legacy-immediate operand checker now enforces register banks and
encodable immediate ranges for 27 mnemonics on both x86 targets. PSHUFW is
MMX-only; PALIGNR accepts matching MMX or XMM operands, and its MMX encoding
must omit the XMM mandatory prefix. Other covered rows require XMM operands.
The i686 dispatcher now reaches SHA1RNDS4, MPSADBW and both GFNI affine rows.

PSHUFW's opcode table and description specify MMX registers, but its REX.R
paragraph refers to extra XMM registers. The table, MMX operation and cached
GNU assembler agree; that contradictory paragraph must not authorize XMM
PSHUFW. Immediate width is also distinct from assembler source spelling:
GNU 2.47 accepts negative aliases for some imm8 rows but not others. The
checker and generated corpus preserve that row-specific policy rather than
assuming every signed spelling is valid or truncating arbitrary integers.
`generate_legacy_immediate_shapes.py` generates 4,590 cases per mode, covering
register classes, mixed banks, unavailable registers, memory destinations,
memory sources, and immediate boundaries. Signed-source acceptance comes from
GNU differential evidence, not from an SDM claim about assembly syntax.

`__builtin_shufflevector` previously returned packed data while nested vector
consumers and returns could treat it as an address. The minimal two-shuffle
native witness terminates with SIGSEGV before the fix. The implementation now
uses one 16-byte pointer-backed vector value consistently through semantic
typing, aggregate sizing, argument lowering, member/conditional expressions,
and returns. It evaluates both operands once, validates known four-by-32-bit
vector geometry and matching element types, and diagnoses nonconstant or
out-of-range selectors. Single-source permutations use PSHUFD where the x86
SSE2 contract permits it; mixed-source and other-target paths retain scalar
lane gathering. This does not add other vector geometries to the builtin.

Adjacent same-destination register PSHUFD/VPSHUFD instructions compose their
two-bit selectors independently within each 128-bit lane. Mixed legacy/VEX,
memory, masks, changed destination, width changes and intervening effects are
excluded. A legacy identity may disappear only for a self-copy; VEX identity
replacement remains VMOVDQA so upper-zeroing is not silently lost. The rule
also runs after stack forwarding, which is when real frontend chains become
adjacent. Reverse-twice becomes `ret`; reverse-then-rotate becomes one
`pshufd $198` plus `ret`, versus two PSHUFD instructions after the correctness
fix alone. This is an instruction-count result, not a universal timing claim.

Regressions include all 65,536 immediate-pair compositions, barrier and
skip-control checks, a native original/optimized/independent-integer-model
comparison including observable YMM upper lanes, and an 18,432-check typed C
fixture. The latter supports separate compiler/driver translation units and
covers nesting, function arguments/returns, conditional struct members,
floating bit patterns, sizeof, _Generic and operand side effects. Differential
runs use only the cached GCC 16.2, Clang 23.1.3 and binutils 2.47 tools.

### i686 GNU vector ABI follow-up

A freestanding GCC 16.2 driver exposed a separate i686 calling-convention
failure: 16-byte GNU `vector_size` arguments/returns were classified like
struct stack/sret objects instead of XMM vectors when SSE was enabled. The
baseline native witness terminates with SIGSEGV. These GNU vector shapes now
reuse the already modeled single-XMM carrier, including signature CType
retention, direct/indirect calls, argument decomposition and return packing.
Explicit `-mno-sse` instead retains the generic stack/sret convention. LCCC's
existing i686 backend and macro contract have an SSE2 baseline, so the default
ABI remains XMM; an independent `-mno-sse2` denial produces SSE-only carriers,
and `-mno-sse -msse` restores baseline SSE2 unless a direct SSE2 denial is
also present. The effective-feature mirror now publishes those baseline and
denial states to this classifier. Prefetch retains its narrower positive-
grant predicate instead of inheriting the backend's baseline. The separate
32/64-byte GNU-vector MEMORY defect is now addressed in the i686 backend:
explicit target-local vector metadata drives natural caller/callee offsets,
call-site dynamic stack realignment, and a single aligned variadic cursor/copy.
Ordinary over-aligned records retain their measured four-byte layout;
memory-passed GNU vectors use natural alignment, including no-SSE v16.
The alignment-ABI gate now requires every i386 v16/v32/v64 LCCC/GCC pairing to
exit zero, including both stack residues and O0/O2; the former divergence
latch is removed. Hosted acceptance is pending the next run. This is an ABI
finding from current compiler differential evidence, not a calling-convention
claim derived from the SDM.

`check_i686_gnu_vector_abi.sh` requires GCC 16.2.0, Clang 23.1.3 and GNU as
2.47. It links freestanding ELF32 programs with lccc-ld, requiring no 32-bit
libc headers/runtime. Across default, explicit SSE2, SSE-only, SSE2-denied,
profile-denied, SSE-restored and no-SSE modes, it exercises **88 complete
cross-TU configurations** at O0/O2/O3/Os, each retaining all 16,384 vector-ABI
and 18,432 typed-shuffle checks. Default and SSE2 run all 24 mixed-compiler
rows. The other five modes run eight GCC/LCCC-family rows; reduced-ISA Clang
self-oracle rows are explicitly excluded because Clang 23.1.3 failed its own
SSE-only `pick_variadic` probe with exit 8 and, after that probe was removed,
its own `iddouble` probe with exit 4. These are recorded upstream-oracle
incompatibilities, not LCCC passes or LCCC failures; no LCCC/GCC probe is
omitted. LLVM 23.1.3's `X86CallingConv.td` passes only `v4f32` in XMM with
SSE1 but no SSE2, whereas GCC/LCCC also use XMM for the tested integer/double
vectors, and Clang's no-SSE vector return rule differs from GCC stack/sret.
The earlier source-only inference that Clang's `-mno-sse -msse` remains
SSE-only was disproved by the hosted 23.1.3 executable probe: it defines
`__SSE2__` after that ordered sequence, so the gate now expects SSE2 for all
three compiler families in `sse-restored`. Linking incompatible families is
not a valid differential test; all compatible mixed rows and all GCC/LCCC
reduced-ISA rows remain mandatory.
Compile-time `__SSE__`/`__SSE2__`/`__SSE3__` checks and non-vacuous PSHUFD/XMM
assembly ceilings cover states that a host runtime cannot observe.
Integer element widths 8/16/32/64, double bit patterns, indirect calls,
register exhaustion, 16-byte stack alignment and variadic named/unnamed
vectors are covered. The shared typed fixture keeps its original checks;
only its libc includes are replaced by freestanding declarations. The
unsharded differential workflow runs this gate after a swap-enabled fastbuild
with the pinned compiler and assembler oracles.
