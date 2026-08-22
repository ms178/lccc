# Session 45 — final red-team audit of SIMD temp promotion (adopted 2026-08-22)

Base: `ms178/lccc` main `58dfe665` (PR #179); merged onto `813766c` (PR #180,
which upstreamed session 44). Build: `fastbuild`, Rust opt-level 1, no LTO,
two jobs, 4 GiB swap. Host: constrained VM without PMU.

> This is Agent B's audit of the session-42/43 vector-temp-promotion
> implementation, adopted and independently red-teamed on top of the
> session-44 soundness sweep (constant-address load sources, lcccsimd
> immediate-arity fix).  The three additional audit fixes applied during
> adoption are listed in "Adoption audit" below.

## Verdict

GLM v2 and the session-42/43 implementation contain important improvements,
but they were not yet production-complete. The central architecture is worth
keeping: exact/conservative alias graphs, source-span-preserving compaction,
per-argument vector signatures, local-allocation escape facts, volatile gates,
and source/slot clobber checks are all substantially better than the original
517-line pass.

The audit found additional semantic holes and one real-workload code-quality
regression. This revision fixes them without regressing the latest-upstream
SIMD matrix. It also removes five instructions from gzip's hot PCLMUL kernel
and two from the multi-definition vector corpus.

No hardware-cycle claim is made. Dynamic instruction evidence comes from
Callgrind, which is deterministic and PMU-independent.

## What GLM got right

- A forwarded load must be invalidated by writes which may alias either its
  source or its destination slot.
- SSA value inequality is not a no-alias proof; GEP/copy/cast, folded pointer
  arithmetic, select and phi matter.
- Offset vectorizer loads cannot be forwarded by substituting the base alone.
- A pointer-backed vector argument needs a per-position width/signature; scalar
  and immediate positions must never be rewritten as vector addresses.
- Volatile and semantic-volatile homes are ineligible.
- `vector_result_width()` is necessary for full-result promotion.
- `for_each_used_value` avoids analysis-time operand-vector allocation.
- Removed instructions and `BasicBlock::source_spans` must be compacted with
  exactly the same mask.
- Store destinations and non-temporal destinations need separate alignment
  semantics; `movntdq`/`movntpd` can safely relax 32-byte alignment to 16.
- Unknown IR/intrinsic forms must fail closed.

## Remaining defects found and fixed

### 1. Aligned AVX contracts were discarded

`Load256` was classified as an unaligned-safe vector argument and `Store256`
as an unaligned-safe destination. The current backend happens to stage these
through generic `vmovdqu` helpers, but the source operations are the aligned
AVX forms. Removing their 32-byte guarantee would make a future direct
`vmovdqa` lowering fault on a program that was defined before this pass.

Fix:

- `Load256` remains a full-width producer, but is not a forwarding consumer;
- its source requires alignment 32;
- `Store256` destinations require alignment 32;
- tests pin both the forwarding and alignment behavior.

### 2. Full vector width was mistaken for write-only behavior

The FMA accumulator forms read and write `dest_ptr`. Redirecting such an
operation from a temporary to the memcpy destination changes which old value
is accumulated. `vector_result_width()` cannot prove a full overwrite.

Fix: promotion now additionally requires
`intrinsic_overwrites_full_result(op)`. Read-modify-write destination semantics
are centralized in `intrinsic_dest_reads_old_value()` and reused by liveness.

### 3. A constant load source was treated as impossible to clobber

A fixed numerical address is still reachable by another pointer. Retaining a
forward across an unknown store can therefore move the read past a write.

Fix: any potentially relevant write invalidates a constant-address source
unless a symbolic object proof exists (none exists for `Operand::Const`).

### 4. Pointer-root convergence missed loop recurrences

The repeated scan required every phi input to have a root before assigning the
phi. It can never solve the common recurrence:

```text
p = phi(initial_parameter, p + stride)
```

It was also O(rounds × instructions).

Fix: dependency rules plus iterative Kosaraju SCC decomposition and a worklist.
A component with exactly one external seed is solved; a multi-seed pointer
cycle stays unknown. No recursion is used, so adversarially long IR cannot
exhaust the Rust stack. Recurrence-derived values are retained for the cost
model.

### 5. Write-only destinations kept dead loads alive

The dead-load sweep counted every non-load intrinsic `dest_ptr` as a read.
Earlier `setzero` initializers consequently kept later, fully forwarded loads
alive. GNU gzip's PCLMUL loop paid four redundant vector load/store round trips:
+128 bytes of frame and +5 static instructions in the hot function.

Fix: count intrinsic arguments as reads, normal destinations as writes, and
only audited read-modify-write destinations as reads. Load SSA results are
still independently required dead before removing an instruction.

### 6. Multi-use Loaddqu forwarding needed a recurrence-aware cost model

Always allowing the established SSE exception is good for adler32 unpack
chains and avoids deferred-register spill explosions. Extending it to a source
computed from a loop-carried pointer, however, lengthened the recurrence's GPR
lifetime and added two static instructions in `vector_defer_multidef_slot`.

Fix: retain the multi-use Loaddqu exception only for non-recurrence-derived
sources. Single-reader loads remain forwardable regardless. This policy was
selected from generated-code and Callgrind A/B evidence, not intuition.

### 7. Wide scalar accesses lost target-natural alignment

Alignment relaxation treated every scalar load/store as requiring no
alignment. On non-x86 targets I128/U128/F128 accesses may require 16 bytes even
when the vector object no longer needs 32.

Fix: preserve natural alignment above the ordinary 8-byte stack baseline.
Narrow scalar and safe GEP paths still relax to ordinary alignment; wide scalar
paths relax 32 → 16.

### 8. Pass ordering used a stale use set

Alignment relaxation ran before forwarding. A load removed immediately later
could therefore keep unnecessary alignment alive.

Fix: the final order is promotion → forwarding → alignment relaxation.

### 9. Result-use lookup was quadratic in pending loads

Every visited value scanned `load_result_slots.values()`.

Fix: construct an `FxHashSet` of load results once and use O(1) membership.

## Corrections to claims in the supplied reports

- `Loadldi128` is not forwardable as a full 128-bit load: it reads 64 bits and
  zeroes the upper half. It is only alignment-safe.
- The current backend's accidental unaligned implementation of an aligned
  intrinsic is not permission to erase the intrinsic's alignment contract.
- The attached “production file” itself does not compile against current IR:
  examples include stale/nonexistent fields and variants such as
  `Memcpy::volatile`, `GetElementPtr::indices`, `CallInfo::callee`,
  `AtomicXchg`, and `AtomicCAS`.
- Empty structural test bodies are not coverage. The committed tests construct
  real current-IR instructions and assert transformed IR.
- The report's 14700KF PMU numbers have no commands, raw samples, revisions, or
  retained artifacts and are not reproducible evidence.

## Validation

### Compiler and tests

- `cargo check --profile fastbuild --locked -j 2`, `-D warnings`: pass.
- Final ship build via `scripts/build_lccc_o1_j2.sh` (release, Rust `-O1`,
  thin LTO, exactly two jobs): pass; release compiler reproduces the 875-
  instruction PCLMUL result.
- Focused module tests: **31/31 pass**.
- Full library tests: **1041 passed, 6 ignored, 0 failed**.
- Ten targeted SIMD/vector C programs compile and execute successfully.
- Two `-g` SIMD builds execute and have decodable line tables.
- Full regression driver: **387 pass, 3 fail**. All three failures reproduce
  with untouched upstream/baseline compilers and are pre-existing:
  `segment_fill_copy_alias`, `check_redundant_test_elimination`, and
  `check_unary_rmw_copy_propagation`.
- `cargo fmt` was intentionally not run per harness requirement.

### Generated-code A/B versus latest upstream `58dfe665`

| Corpus | upstream instructions | audited instructions | delta |
|---|---:|---:|---:|
| gzip `crc32_update_no_xor_pclmul` | 880 | **875** | **-5** |
| `vector_defer_multidef_slot` | 565 | **563** | **-2** |
| `simd_avx2_256` | 734 | 734 | 0 |
| `simd_avx2_defer_chain` | 121 | 121 | 0 |
| `simd_crc_adler` | 1371 | 1371 | 0 |
| `simd_new_hw_ops` | 927 | 927 | 0 |
| `simd_movnt` | 390 | 390 | 0 |
| `simd_sse2_arith` | 1041 | 1041 | 0 |
| `simd_insert_extract` | 994 | 994 | 0 |

The `simd_crc_adler` Callgrind count is exactly unchanged at **51,407,853**
whole-process instructions (main and `adler32_simd` also individually exact),
which caught and rejected an intermediate static-code “improvement” that added
897 dynamically executed instructions.

### GNU gzip 1.14 real workload

Both upstream and audited compilers:

- pass **30/30** upstream gzip tests;
- pass deterministic compression/decompression round trips;
- produce identical compressed bytes in the Callgrind treatment.

Executable text falls from **102,405 to 102,373 bytes** (-32). On a deterministic
256 KiB level-1 compression input, PMU-free Callgrind instruction count falls:

```text
upstream: 12,072,624
patched : 12,052,389
change  :    -20,235  (-0.168%)
```

Separate-run pinned VM medians were generally slightly lower after the patch,
but are too noisy/non-paired to claim as a runtime speedup.

### zlib-ng gate

The pinned zlib-ng 2.3.3 gate reaches the same pre-existing frontend/header
blocker: `_mm_shuffle_pd` expands to a three-argument call while
`__lccc_simd128_ps_shufpd128` is declared with four. The failure is unrelated
to this pass and is recorded rather than hidden.

## Adoption audit (session 45, on top of the session-44 sweep)

The following issues were found while re-red-teaming this patch against the
already-upstreamed session-44 base and fixed before delivery:

1. **Module-wide array sizing in the SCC machinery.** `node_count` was derived
   from `func.max_value_id()`, which is the MODULE-wide value counter — a
   function late in a large TU allocated adjacency/component arrays
   proportional to the whole translation unit. Value ids are now remapped to
   a dense per-function range before the Kosaraju decomposition; only rule
   dests/sources are indexed.
2. **Quadratic result-use lookup survived in the terminator loop.** The
   instruction sweep was converted to the O(1) `load_results` set, but the
   terminator sweep still ran `load_result_slots.values().any(...)` per used
   value. It now uses the same O(1) set.
3. **`cyclic_seed` construction** was an obfuscated
   `(len == 1).then(..).flatten()` double-Option; replaced with a plain
   conditional.
4. **Complexity comment corrected.** The worklist refires a merge rule once
   per newly-known source, so the honest bound is `O(V + Σ arity²)` — linear
   for real IR — not a flat O(V+E) as originally documented.

The constant-address load-source rule from the session-44 sweep was kept
instead of this patch's unconditional `false`: the alloca-confined write
exemption carries a symbolic-object proof on the WRITE side, so retaining the
forward there is both sound and strictly more precise.

## Follow-up work

1. Move vector argument widths, address/immediate roles, destination access
   mode, and required alignment into one authoritative `IntrinsicOp` signature
   table shared by lowering, optimizers and backends.
2. Represent memory effects centrally; exhaustive local matches are safe today
   but expensive to maintain across every pass.
3. Add cross-block forwarding only with dominance plus memory-SSA/equivalent
   clobber reasoning and a register-pressure/lifetime cost model.
4. Extend vector result width/protection coherently before considering 64-byte
   AVX-512 homes. Raptor Lake desktop is not the AVX-512 target.
5. ~~Fix the zlib-ng shuffle signature mismatch~~ **DONE in session 44**
   (`gen_lcccsimd.py` immediate-builtin prototypes no longer declare a
   trailing `int __imm`; t128/t256/t512 intrinsic suites compile) — re-run
   the complete zlib-ng CTest and round-trip gate on the current tree.
6. ~~Fix the three pre-existing regression failures named above~~
   **RESOLVED**: `segment_fill_copy_alias` was a host-environment issue
   (missing i386 loader); `check_redundant_test_elimination` and
   `check_unary_rmw_copy_propagation` pass on the current tree.
7. Validate the -0.168% deterministic instruction reduction on the requested
   i7-14700KF with paired randomized runs, fixed P-core affinity/governor, and
   PMU top-down counters.
8. Continue the Compiler Explorer gap work in the appropriate scalar passes;
   the retained GCC/Clang/ICC/ICX oracle reports 117 static instructions for
   LCCC versus GCC's 49 on the whole gzip CRC corpus. Do not overfit
   vector-temp promotion to that unrelated scalar gap.
