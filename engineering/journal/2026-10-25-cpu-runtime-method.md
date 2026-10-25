# CPU-local runtime follow-up

The earlier Intel VM result is not an AMD EPYC result. The supplied CI report
uses the correct merged compiler, GCC as its only reference, and portable
`-O2 -std=gnu17`, not `-march=native`. The local compiler emits identical nbody
assembly with and without the explicit GNU17 spelling. Thus neither a stale
compiler nor that dialect flag explains the discrepancy. A before/vendor ratio
from one machine cannot be compared to an after/vendor ratio from another as
if it were a paired speedup. CI correctness success is not a performance win.

The shard merger also retained the first shard's machine metadata for every
row. This is a real provenance/reporting defect, not an explanation that makes
the nbody gap disappear. Merged rows now identify their shard; each shard keeps
its complete machine/affinity/PMU metadata; Markdown shows per-row CPUs and
actual optimization flags. Cross-host aggregates are explicitly qualified.

The opt-in research lane now builds an archived, immutable
baseline and the candidate with the same Rust toolchain, then measures
before/current/GCC/latest Clang on the SAME host, serially, for both portable
and native settings. Canonical workload sources/sizes and required per-workload
flags are retained. Three excluded warm-ups, randomized paired rounds, output
checks, binary/source hashes and original samples accompany the report. CPU
identity is observed, not guaranteed by the hosted runner label. Never describe
an Intel allocation as an AMD allocation. An explicit native profile complements
rather than replaces portable coverage.

Short-kernel follow-up uses the existing fresh-child calibrated entry runner,
excluding exec startup. It compiles the original main, verifies an ELF-preserving
entry-symbol rename, uses one common driver and checks repeated outputs. It
explicitly deduplicates byte-identical linked binaries: aliased timings are not
independent A/A samples, and no confidence interval is invented for them.
Callgrind remains a separate bounded portable instruction/cache/branch diagnostic,
not a native-cycle model. Research summaries and SQRT disassembly excerpts are
published through check annotations because the sandbox cannot reach the blob
storage host used for Actions artifact downloads; full evidence remains in the
uploaded artifact.

A separate candidate replaces a distinct legacy scalar-SQRT destination seed
with XORPS/XORPD plus source-direct SQRT. It preserves aliased inputs and VEX
forms and uses only SSE1 for F32. This is a hypothesis under measurement, not a
claimed cross-CPU improvement. Retain it only if meaningful paired results and
correctness justify it. The next generic witness is uniform private-global
pointer arguments: IPCP currently propagates the uniform trip count of the
noinline F32 map, but not the uniform buffer addresses; this withholds global
alignment and packed memory-operand folding opportunities. TLS, escape,
interposition and alias safety must be proved before implementing that change.

Provisioning found a second limitation: main's installer requests Clang 23.1.4,
but the transport tag still contains 23.1.3. The restore correctly refuses this
mismatch. Do not use that older compiler under the new label. Hosted research
uses the current official 23.1.4 installer; local reference work uses verified
GCC 16.2, binutils 2.47 and mold 2.42.1. Local Rust is the verified 1.97 MSRV
cache; hosted baseline and candidate use the same current stable Rust.
An 8 GiB swap file is active; builds use warning-denied fastbuild and two jobs.
No local CI sweep is run.

The integration cannot dispatch Actions manually (403 authorization). The
research definition therefore also opts in PRs that explicitly change that
workflow file; ordinary code-only PRs do not acquire an extra research build.
