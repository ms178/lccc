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
