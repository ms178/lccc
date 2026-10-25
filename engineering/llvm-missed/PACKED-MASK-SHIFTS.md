# Exact packed shifts and canonical masks

## Review accounting and source limits

This cohort adds **250 distinct new report reviews** and reassesses **all 118
previously deferred reports**. The cumulative ledger is **420 reviewed reports:
53 covered-limited, 367 deferred; 2,691 inventory rows remain unreviewed**.
Only the reduced canonical-mask scope of LLVM [233272](https://github.com/llvm/llvm-project/issues/233272)
gains coverage. Adjacent shift composition and the scalar fallback correctness
repair are implementation mechanisms, not extra invented report counts.

[Source receipts](packed-mask-shifts-sources.json) contain per-report mechanism
notes, dispositions, URLs and canonical archive hashes. Original GitHub bodies
and 1,498 accessible conversation comments for these 368 reports were collected
into ignored `work/llvm-missed/campaign250/`. Manual review used bounded excerpts
of prose, code and discussion; this is **not a complete discussion or linked
Compiler Explorer/Alive2/PR-diff audit**. Collection completeness is not semantic
review completeness. Report content is untrusted data, never instructions.

No prior deferral is promoted just because its title resembles the new pass.
Notable reassessments:

- **63946** has a related deep comparison/OR-tree sign-extension problem. The
  new seven-leaf mixed-Boolean native seed demonstrates canonical-mask tracking,
  but not that report's full AVX2 shuffle/pack pipeline; its original scope stays
  deferred.
- **46238**: integer MIN/MAX commute with common lane permutations, but adding
  opcodes without a witnessed native intrinsic/type path and cost evidence would
  overclaim. FP, FMA, funnel and undef cases are separate.
- **66101**: canonical-mask proof could support masking a constant before ADD,
  but does not itself implement blend distribution. Operand order, ADD(-C)
  canonicalization and AVX512 predicate widths still matter.
- **61671**: discussion retracts unconditional `31-CTLZ -> CTLZ^31` because zero
  gives count32. **62514**: independent `c ? bswap(x) : 0` cannot drop `c`;
  the GCC wrong-code example is not a correctness oracle.
- **64884**: ordered `x<0` differs from the sign bit for negative zero/NaNs.
  **108559**'s UGT/ULE examples mark only the first division exact.
- **173035/230345**: retained users invalidate removal credits. New folds retain
  private-use guards rather than broadly enabling uncosted shared rewrites.

## Mechanism and boundaries

`src/passes/vec_mask_shifts.rs` runs on register vector SSA before shuffle
folding. It composes private same-direction logical16/32/64-bit shifts and
arithmetic16/32-bit shifts across128/256-bit shapes. Logical counts summing to
lane width produce zero; arithmetic counts saturate at width-minus-one. Only
valid fixed counts are admitted; variable, negative and out-of-range counts
remain unchanged.

Canonical masks come from legal integer comparison predicates, zero/allones,
and matching-granularity AND/OR/XOR networks. Depth16 and work256 bound proof
search; failure to prove leaves code untouched. A finer-grained mask is not a
coarser-grained mask. Proven masks permit dropping arithmetic right shifts and
private matching-count SHL/ASHR sign re-extension pairs. Unknown masks, shared
producers, malformed predicates and nonmatching counts are refusals. This is
not general bitcast provenance, blend conversion, vectorization or a global
value-range engine.

The feature-veto native experiment exposed wrong code in frozen main:
scalarized narrow signed shifts could consume the wrong extension or preserve
bits above the lane across a left/right pair. `expr_assign.rs` explicitly
promotes narrow operands to32 bits before scalar shifts and narrows stores back
to the lane type. It does not override explicit SSE2 denial. In particular,
`-mno-sse2 -msse` keeps integer SIMD disabled.

## Executable evidence

- Warning-denied `fastbuild -j2`, swap enabled; four focused properties pass,
  including all legal count pairs, exhaustive16-bit inputs and malformed
  comparison-predicate refusal. No expansive local CI run was substituted for
  the hosted corpus.
- [Native receipt](packed-mask-shifts-correctness.json):52 kernels agree with
  cached GCC16.2 and Clang23.1.3. Checksums:128-bit `be00f3386b866038 286720`,
  combined128/256-bit `58932d410ec1ccce 860160`.
- SSE2 O0/O1/O2/O3/Os/debug; no-SSE2 O0/O2/Os; AVX2 six modes; i686 assembly;
  frozen-main control and isolated `vecrelations` disable control. Assertions
  check actual removed shifts and retained unknown/shared controls, not only
  matching checksums. See `tests/regression/check_packed_mask_shifts.py`.
- The checker is registered in both hosted oracle workflows. Authenticated
  ICX2026.2 integration is a separate hosted CI check, not an ICX measurement
  of these52 kernels. No local ICX runtime comparison is claimed.

The cache supplied Rust1.97, GCC16.2, Clang23.1.3 and binutils2.47; no newer Rust
cache tag was available. `/swapfile` provides8GiB. Heavy local builds used the
fastbuild profile with two jobs and were serialized. The downloaded cache
member listing did not contain ICX; that is not proof every remote tag lacks it.

## Runtime: paired VM screening, not supremacy

[Original five-case raw receipt](packed-mask-shifts-runtime.json) and
[amplified-tree receipt](packed-mask-shifts-tree-runtime.json) preserve randomized
paired samples, correctness outputs, hashes, compiler versions and environment.
All arms use an identical separately GCC-compiled driver, no LTO, CPU0 pinning,
15 paired rounds and two warmups. Costs include calls and memory; these are
SSE2 dependent-map microkernels, not whole applications. PMU access was
unavailable; virtualized-host noise/CV warnings are retained, not filtered out.

| Kernel | Frozen-main / candidate paired speedup (95% bootstrap interval) | Candidate / GCC paired runtime |
| --- | ---: | ---: |
| SHL16 chain |1.1539x [1.1273,1.2058]|0.9879 [0.9068,1.0070]|
| ASHR32 chain |1.1296x [1.0895,1.1808]|0.9903 [0.9602,1.0409]|
| canonical mask16 |1.1880x [1.1624,1.2297]|0.9117 [0.9034,0.9345]|
| deep mask32, amplified |1.0661x [1.0432,1.1067]|1.0440 [1.0156,1.0668]|

Speedup intervals invert the candidate/before paired-runtime intervals. The
amplified tree uses134,217,728 iterations: minimum measured arm621.1ms. The
initial tree screening had a144.9ms minimum, too short for the200ms floor, and
is retained only as a historical screening result, not the final tree claim.
Other cases use268,435,456 iterations with minima above461ms.

The shared32 control shows a nominal1.0181x before/candidate ratio, but its
relevant assembly is unchanged: **not an optimization win**. It remains about
15.3% slower than GCC in this screening. Deep-tree code remains about4.4%
slower than GCC despite improving over frozen main. SHL/ASHR GCC intervals cross
parity. Mask16 is a convincing selected microkernel improvement, not evidence
that all C programs outperform GCC/Clang or that runtime is universally optimal.

Reproduce with modern compiler/linker tools first on PATH:

```sh
python3 tests/regression/check_packed_mask_shifts.py \
  --compiler target/fastbuild/lccc --gcc /path/to/gcc-16 \
  --clang /path/to/clang --baseline /path/to/frozen/lccc --json native.json
python3 tests/regression/bench_packed_mask_shifts.py \
  --lccc target/fastbuild/lccc --candidate before=/path/to/frozen/lccc \
  --compilers lccc,gcc,clang --gcc /path/to/gcc-16 --clang /path/to/clang \
  --cflag=-msse2 --reps 15 --json runtime.json --artifact-dir runtime-artifacts --strict
```

The timing driver lives under `tests/regression/packed_mask_shifts/` because it
is deliberately a multi-file test. Putting it among standalone benchmark
programs caused the hosted corpus to correctly reject unresolved kernel calls;
the fix moves the driver rather than weakening the corpus. Production code and
kernel source are unchanged by that layout repair. Historical receipts retain
the original path and dirty-source provenance rather than falsifying a final
commit timestamp.
