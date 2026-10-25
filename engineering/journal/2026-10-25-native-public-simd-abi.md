# Native public SIMD types and the i386 AVX boundary

The public `__m128`, `__m256`, and `__m512` float, double, and integer
families now use GNU vector types rather than records containing `__val`.
Their natural alignment follows the requested ISA, as in GCC 16.2.0; the
`_u` aliases alone override alignment to one. Native lane access, vector
literals, equal-width bitcasts, arithmetic, and arrays of vector objects
are supported by the public definitions. MMX remains a separate change.

Private pointer-result operations now declare pointer results, not public
aggregate results. Scalar mask comparisons and EXTRACTPS retain scalar
returns. Float and double AVX lane operations have distinct private typed
names even when their instruction selection is identical.

The i386 calling convention shares three vector argument register numbers
across XMM/YMM/ZMM widths. Requested AVX/AVX-512 ABI levels select the wide
GNU vector carriers; without those features the stack/sret convention
remains. Overflow and variadic vector arguments retain their natural stack
alignment. Wide returns use the shared internal result-buffer shape but
consume no hidden ABI argument and do not cause `ret $4`. Callee register
homes use the effective address of over-aligned allocas. Indirect calls
consult the same vector classification as direct calls. Local function
pointer signatures retain `...` and parameter CTypes.

Arrays of GNU vectors now use the existing vector-object initializer
planner, including whole-vector expression copies. Previously the scalar
array path wrote an address into each element instead of its vector bytes.

The public ABI oracle compiles each translation unit against its compiler's
own headers and links every caller/provider pairing using GCC 16.2.0.
Both i386 and x86-64 passed 72 executable legs each: float/double/integer,
SSE4.2/AVX1-without-AVX2/AVX2, O0/O2, nine-vector register pressure, mixed
XMM and wide arguments, GP and 64-bit scalars, direct/volatile indirect
calls, and named/unnamed variadic vectors. Every byte is checked across 17
lane-distinct payloads. No missing-ISA or multilib skip is accepted.

The local rehydrated Rust 1.97 cache is bootstrap evidence, not latest-stable
acceptance: GitHub's moving stable job provides that acceptance. Swap was
active and builds used fastbuild with two jobs. The independent generated
header contracts passed nine Python tests; native public type/operator
execution and the VTEST/widening guard-page witnesses also passed locally.
The hosted public ABI step is mandatory before runtime/Callgrind comparisons.

This is not a claim of exhaustive GNU raw-builtin typing, completion of the
full manual audit, expert approval, or a measured performance improvement.
Those require their own implementation and current hosted evidence.

## Canonical GNU result follow-up

The checked-in GCC 16.2 type catalog now has 233 canonical names with complete
function descriptors and mode availability. One generator supplies native CTypes
and a C oracle: 233 result assertions on x86-64, 228 on i686 (five genuinely
64-bit-only names). Twenty post-AVX2 VNNI operations have type contracts only;
the executable witness covers 191 vector-returning operations. All 191 outputs
match real GCC 16.2 byte-for-byte on both modes at O0 and O2. Ninety-two additional
canonical spellings reuse existing native emitters; dynamically registered
widening names remain owned by their shared table rather than duplicated.

Raw 128-bit results now share object-address IR carriers with native 256-bit
results; no packed-I128 exception remains. Public conversion helpers use vector
casts, not pointer dereferences of native results. Fifty-two AVX2 wrappers now
call canonical native builtins with explicit input lane casts instead of scalar
C loop implementations. Immediate wrappers are macros so their arguments remain
visible to immediate validation during lowering.

GNU byte-shift/alignment builtins take bit counts. Public byte counts are
multiplied by eight, and lowering divides only valid constant multiples of eight
in 0..2040. Negative, non-byte-multiple, and oversized raw counts are diagnosed
rather than wrapped. Forty runtime edge results (0, 1, 15, 16, 17, 31, 32, 255
public bytes) match GCC on both modes at O0/O2. A separate negative oracle checks
35 invalid count/mode cases per compiler without allowing ICEs.

The hosted frontend failure was an obsolete test requiring rejection of
`__m256` indexing under its former struct representation. Its replacement
retains all nine truly invalid-subscript rejections and executes public native
128/256 indexing against GCC at O0/O2. Generated arity metadata now includes
public function-like macros, canonical facts, and dynamically registered AVX
families; 558 intrinsic arities are pinned. CI/local command parity is 399/27;
the local CI orchestrator itself was not run. New exact-tool dual-mode research
steps require both GNU fixtures to execute without skips.

The final local build still uses the documented cached Rust bootstrap; current
stable acceptance and the full matrix belong to GitHub. This does not establish
full GNU argument-signature enforcement, exhaustive remaining API completion,
new performance gains, latest-manual verification, or expert approval.

Final follow-up fastbuild completed in 24.63 seconds, with swap active and two
jobs. The rebuilt binary again passed the 35-case diagnostic oracle and public
subscript oracle. Fresh public cross-compiler ABI grids passed 72 executable
legs per mode, 144 total, without skips; expanded GNU result outputs were also
rechecked against both GCC binaries after the final arity-table rebuild.
