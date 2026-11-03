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

## Scoped warning gate evidence (2026-11-01)

At revision `0dce6e6c7`, replaying **554 actual saved C compilation commands**
from init/kernel/mm/arch/x86 with only the compile/output/dependency switches
replaced by `-fsyntax-only` passed. This is not a code-generation, linker,
whole-tree or QEMU result. The earlier rstat BPF failure is fixed by honoring
GCC diagnostic push/ignore/pop scopes, not by exempting weak/BPF functions.
The boot-only rebuild using that compiler independently passed at 31,024 bytes,
with 24 ISA-clean objects and a GNU-linker byte-identical flat image. Its size
is unchanged from the previous locally validated boot gate; no new size win
is inferred from these diagnostic changes. Microsoft fixed/record/variadic
interoperability gates also passed again.

An inventory of **2,478 saved LCCC object commands**, including drivers already
reached by the ongoing full build, found 56 warning-option spellings. Testing
those spellings on an empty non-kernel TU in strict option mode still rejects
these 16 positive spellings:

- `-Wenum-conversion`, `-Werror=date-time`, `-Werror=designated-init`
- `-Wframe-larger-than=2048`, `-Wvla-larger-than=1`, `-Wimplicit-fallthrough=5`
- `-Wformat-overflow`, `-Wformat-truncation`, `-Wmissing-format-attribute`
- `-Wmissing-include-dirs`, `-Wold-style-definition`, `-Wpacked-not-aligned`
- `-Wrestrict`, `-Wstringop-truncation`, `-Wunused-but-set-variable`,
  `-Wunused-const-variable`

This is an observed saved-command subset, not a complete warning audit or
proof that every accepted negative spelling has an implemented diagnostic.
Unsupported optimization/security/debug options also remain visible. These
warnings must not be hidden by deleting flags or relabeling no-ops as support.
Final full-kernel link/boot, warning-free acceptance and measured runtime and
code-size improvements remain outstanding.

### Follow-up: designated initializers and stack-size diagnostics (in progress)

The current working implementation replaces tolerance for `designated_init`,
`-Wframe-larger-than=N`, and `-Wvla-larger-than=N` with diagnostics. This is
not completion of the warning/optimization inventory or the full-kernel gate.

Engineering choices:

- Record definitions own the designated-initializer requirement. A forward or
  reference-only attribute does not retroactively mark a later/existing
  definition (GCC 16.2 non-kernel oracle). Invalid scalar-member attributes are
  rejected and cannot leak into the enclosing record. Initializer source spans
  allow pragma state to be applied at the initializer, not its type definition.
- Fixed-frame measurements come from each backend's actual frame reservation,
  rather than a frontend sum of local C objects. Consequently exact sizes are
  backend-specific; differential fixtures compare separated small/large cases,
  while backend probes independently check the reported reservation.
- Dynamic allocations retain VLA source provenance through IR transformations;
  plain `__builtin_alloca` is not mislabeled a VLA. Optimized SSA range reasoning
  uses only dominating single-predecessor edges. A dominating join alone cannot
  establish which branch was taken. Typed arithmetic, checked inverses and
  bounded shift reasoning must not infer a small bound from wrapping or
  narrowing operations. Analysis limits (16 recursive levels, 256 work units per
  range query, 128 ancestor steps, 8 inverse steps) give conservative warnings
  rather than unbounded diagnostic work. More complex joins/loops can therefore
  still produce conservative false positives.
- GCC's numeric thresholds saturate on decimal/unit overflow. `warning`/`error`
  pragmas require a valid number; `ignored` pragmas intentionally accept an
  empty or invalid suffix. Scoped suppression applies to both frame and VLA
  families. `-Og` retains a distinct diagnostic policy: GCC suppresses VLA range
  warnings there, unlike `-O1`. A syntactic VLA with an optimized constant
  extent still warns at the analyzed optimization levels; it must not be
  confused with a source-level fixed array.
- Ordinary compilations without these warnings or their pragma names avoid
  building a second diagnostic scope table. No runtime or compiler-speed gain
  has been benchmarked from this change, and none is claimed.

The validated checkpoint passed 696 designated-init, 1,747 frame-size and
6,336 VLA-size checks across all four backends against non-kernel GCC 16.2.
The VLA gate covers six optimization modes and last-option-wins transitions
between `-Og` and `-O1`, wrapping/narrowing guards, constant bounds, numeric
thresholds, scoped promotion/demotion, and suppression. Its pragma/pop functions
have distinct volatile effects so GCC IPA-ICF cannot delete the allocation
whose reporting state is being tested; a separate `-fno-ipa-icf` non-kernel
probe confirmed the original oracle artifact before correcting the fixture.

Fastbuild library tests passed **7,696 with zero failures**; the existing ten
ignored tests remain unmeasured. All-target Clippy passed with warnings denied.
Attribute, qualifier, diagnostic-precedence/pragma, date/time, layout, enum,
forward-enum ABI and verbose-assembly gates also passed. The audit census did
not increase. The warning-compatibility precision sweep still reports two
compile-divergence skips and seven GCC-only known-gap lines; these are not
counted as newly validated support. The three new CI gates have separate named steps and measured
costs (4.77/18.56/78.16 seconds locally); the existing lane-split tool repartitions
all 156 frontend steps without dropping coverage. Predicted frontend lane costs
are 352.4/352.3/352.3 seconds using the retained old costs plus these three new
measurements; this is not a new measurement of hosted wall-clock balance.

The LCCC-only Linux 6.18.58 boot rebuild passed at **31,024 bytes**, with 1,744
bytes of headroom, 24 ISA-clean objects and GNU-linker flat-image equality. GCC
compiled no kernel target code. The size is unchanged from the prior 31,024-byte
boot checkpoint: no code-size or runtime improvement is claimed. Full kernel
link, objtool and QEMU boot are still not established for this working tree.
The six remaining warning families and outstanding optimization options remain
acceptance requirements; this checkpoint is not task completion.
