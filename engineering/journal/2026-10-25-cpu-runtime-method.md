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

## Follow-up findings

The first local 15-pair SQRT study does **not** establish a gain: portable nbody
was 0.9968 after/before (95% bootstrap interval 0.9825–1.0013), spectral norm
1.0042 (0.9900–1.0109). The other ten portable and all twelve native executables
were byte-identical before/after; their timing variation is not an optimization
result. Hosted evidence is still pending.

The private-address experiment now additionally removes a whole closed
signature only when every formal is specialized and at least one is a private
address. It does not renumber partially live signatures. It excludes exported,
escaped, weak, TLS, aliased, symver, assembly-visible, runtime-hook and ABI/frame
introspection cases, mismatched/variadic/sret calls, and fastcall/regparm pruning.
Mutable object contents remain mutable. Three targeted unit tests and a GCC16/
LCCC differential fixture cover positive and refusal cases, argument-expression
side effects, partial signatures and nonzero pointer offsets. The completed 31-pair study rejected this prototype: portable F64 maps regressed
15.05% (after/before 1.1505, interval 1.1354–1.1587), F32 maps 1.52%
(1.0078–1.0218). The native SSE1 result was 0.9864 (0.9808–0.9948), but
small isolated wins do not justify the larger regression. The implementation
was removed, with source and evidence preserved outside the patchset. The C
correctness witness is retained. An unchanged native hot-web executable even
showed a 1.0161 timing ratio with an interval excluding one: finite-round VM
bootstrap intervals are not a guarantee against false discoveries or drift. No legacy
SSE memory fold was added: backend placement alignment is not itself a proof
that a particular access is 16-byte aligned.

The first hosted contract failure was in research workflow wiring, not compiler
correctness. GAS now uses the contracted default installation path. Hosted-only
measurement/publication commands have explicit reasons; ELF, output and report
rejection contracts remain mirrored by corpus discovery. The publication unit
suite was moved into that discovery rather than exempting its correctness checks.
The targeted static parity checker and eleven reporting/provenance tests pass.
Research concurrency is revision-specific so a follow-up push does not cancel
an already-running measurement experiment.

The next candidate changes only destructive SSE register-allocation ties: preserve
handoff preference, reservation, interference checks and reuse recency, but prefer
a lower register number on an otherwise exact tie to reduce REX/VEX encoding
bytes. It introduces no alignment assumption or new ISA requirement. Targeted RA
checks and a separate serial paired study are running. This is not yet a measured
speedup. The Callgrind lane now explicitly requests empty target flags: its helper
default was x86-64-v3, which was not the intended portable diagnostic.

## First hosted same-host result

The completed research run allocated **AMD EPYC 7763**, not EPYC 9V74. GCC16.2
and Clang23.1.4 were identified. Portable nbody after/before was 0.997804
(0.997206–0.998014): only 0.22%, not a remedy for the vendor gap and not proof
of a gain on 9V74. Portable spectral norm was 1.000267 (0.999745–1.001021).
Only those two portable binaries changed. All twelve native binaries were
identical. Native binary trees nevertheless timed 7.4% slower and XML scanning
2.5% faster with intervals excluding one: both are timing/system effects,
not source-code effects. This reinforces the need to inspect executable hashes.

| Workload | Portable LCCC/GCC16 | Native LCCC/GCC16 |
| --- | ---: | ---: |
| nbody | 1.3052 | 0.9945 |
| hot web loop | 0.7592 | 0.8628 |
| byte loads | 1.4308 | 1.6743 |
| siphash | 1.3019 | 1.3468 |
| double reduction | 1.2097 | 1.0185 |
| spectral norm | 1.1691 | 1.0075 |
| F32 product map | 1.0160 | 1.0026 |

These are within-profile vendor ratios, not paired native-versus-portable gains;
the two profiles ran serially. Their differences do not transfer across CPU
models. Startup-excluded byte-load results were worse still: GCC/reference
0.5621 portable and 0.4311 native, i.e. LCCC remained substantially behind.
All bounded main/candidate Callgrind Ir ratios were 1.0, consistent with its
x86-64-v3 target leaving the SQRT candidate inactive. Its diagnostic source/size/
target is distinct from the default-size wall-clock study. Complete summary
receipts and report hashes are in the adjacent evidence directory; raw samples
and executables remain in the linked hosted artifact, not these receipts.

The lower-register candidate passed warning-denied fastbuild of both binaries,
147 targeted allocator tests (including the pool-order tie regression), and the
GCC16/LCCC differential C witness. Its portable 31-pair study measured I32 maps
0.9843 (0.9683–0.9957), F32 maps 0.9967 (0.9824–1.0045), and F64 maps 1.0159
(0.9979–1.0246). The unchanged hot-web executable also measured 0.9855 with an
interval excluding one, so the small I32 result alone is not robust evidence
of a source speedup. Assembly confirms the intended mechanism: F64 map
arithmetic uses XMM3/XMM4 instead of XMM14/XMM15, removing four REX bytes per
unrolled hot loop without changing the operations or widening the ISA. Native
and hosted measurements remain pending. Do not advertise runtime gains yet.
