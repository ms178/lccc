# Kernel pattern review and measured ABI optimization

Status: **not a completed kernel optimization audit or a kernel benchmark**.
The full kernel still needs a successful link and QEMU boot. Unsupported-option
warnings are not hidden or treated as implemented optimizations.

## Reproducible source coverage

Run against the actual prepared CachyMod tree, without compiling target code:

```sh
python3 scripts/kernel_pattern_inventory.py --kernel "$KERNEL_DIR" \
  --out /path/outside/git/kernel-pattern-inventory.json
```

The 2026-11-01 run covered **63,656 C/header/assembly files**, including files
with zero matches. It lexically screened 62,268 C/header files and hashed the
assembly files without pretending to interpret them. Each regular file has a
SHA256, byte count and line count. Symlinks are explicitly reported, not followed.
The configuration hash was
`3d06a10a7773a3ca9fff4f425b8adff0cf373c656b849a7202744924034d1e5d`.
The 28-MB report stays outside Git at `/home/user/kernel-pattern-inventory.json`.

This first pass counts inactive preprocessor branches and macro definitions too.
It is **not manual/semantic analysis of every file**, nor does a high count prove
that a pattern is compiled, hot, safe to transform, or profitable. Counts are
review leads, not optimization scores. Sample line lists are capped at six per
pattern; counts themselves are not capped.

| Pattern | Lexical occurrences |
|---|---:|
| Loops | 176,638 |
| Bulk memory operations | 53,745 |
| String operations | 17,919 |
| Single-access macros | 11,758 |
| Atomic operations | 22,230 |
| Barriers | 5,913 |
| Locking/RCU operations | 161,848 |
| Wide integer spellings | 336 |
| Microsoft ABI spellings | 104 |

## Concrete follow-up obligations

These are constraints for further investigation, not claims of landed wins:

- `lib/string.c`: 54 loop and 9 bulk-memory spellings. Establish which generic
  implementations survive x86 architecture overrides before optimizing them.
  Prove overlap, alignment and page-boundary safety before widening accesses.
  Kernel `-mno-sse` forbids simply transplanting userspace vectorization wins.
- `lib/maple_tree.c`: 102 loop, 18 bulk-memory and 111 branch-hint spellings.
  Investigate tagged-pointer address formation and redundant loads; preserve
  node layout and RCU publication ordering. Lexical matching misses abstractions.
- `kernel/sched/fair.c`: 79 single-access, 21 atomic and 139 locking/RCU spellings.
  Arithmetic reductions must preserve signedness, overflow policy and scheduler
  accounting. Do not infer speculation safety from a branch's likelihood.
- `mm/page_alloc.c`: 76 loop and 19 single-access spellings. Distinguish per-CPU
  accesses, zone locking and allocation slow paths before considering store
  motion or loop transformations. Unconditional stores may introduce races.
- `net/core/skbuff.c`: 71 loop and 33 bulk-memory spellings. Fragment boundaries,
  ownership and refcounts constrain combining copies. A memory-call count does
  not identify a profitable or legal vectorization site.
- `drivers/gpu/drm/i915/i915_pmu.c`: the full build exposed nested member
  addresses of array compound literals. This is an initializer/relocation
  correctness issue, not a runtime optimization. The reduction also exposed
  temporary-AST pointer reuse in literal caching and missing multidimensional
  initializer strides. The new fixture tests distinct mutable literal objects,
  macro-expanded occurrences, multidimensional arrays and nonzero field offsets.

A complete semantic review, representative compiled-object comparisons and
workload-level kernel measurements remain owed.

## Landed optimization: skip named Microsoft variadic home slots

The cursor starts after all fixed IR parameters, including an implicit hidden
return pointer. Therefore fixed GP argument home slots cannot be read by that
cursor. Save only the remaining unnamed GP register arguments; retain every
required overflow argument and caller-owned home slot.

Correctness coverage includes O0/O2/Os mixed FP/GP arguments, cursor copies,
helper consumption, hidden returns, five fixed parameters, wide/complex/vector
payload reads, simultaneous SysV lists, malformed inputs, and a no-SSE assembly
witness checking that four fixed GP positions do not produce home stores.

Reproduce a **synthetic non-kernel** comparison:

```sh
python3 scripts/measure_ms_varargs_homes.py \
  --before /path/to/pre-optimization/lccc --after /path/to/optimized/lccc \
  --out /path/outside/git/msabi-home-measurements.json
```

Both target objects use LCCC. GCC 16.2 builds only the non-kernel timing/oracle
harness. The recorded seven alternating rounds each made ten million calls:

| Function | Before bytes | After bytes |
|---|---:|---:|
| Four fixed arguments | 89 | 73 |
| Eight fixed arguments | 139 | 123 |
| Total tested function bytes | 228 | 196 |

Each function lost four redundant stores, **16 bytes**. Median sampled process
CPU time fell from 0.044993835 s to 0.031127175 s; median wall time fell from
0.045161542 s to 0.031204314 s. Raw samples and compiler/source fingerprints are
in `/home/user/msabi-home-measurements.json`. The before compiler hash is
`645d6cc37e60476a4f6f4b0c2baa4f0f4aa546167c0e17336a83c650f024567a`;
the measured after compiler hash is
`affb714175c3e0472f592c8d82b71ffd2e27ff8afb97cc59650b0ce05454bdf3`.

These are local microbenchmark observations, not a statistical-significance
claim and **not measured kernel runtime or whole-kernel code-size benefits**.
Their relevance depends on whether such definitions occur on a real hot path.
