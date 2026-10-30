# S84: retire dead address chains through allocator-owned `%rbp`

## Scope and outcome

S83 is the unchanged baseline (`67d6321839eb13c678b03c7682237e77a997377d`).
The S84 continuation first rechecked scalar-result SIMD extraction. No reachable
producer-less extraction miscompile was established, so the intrinsic
classification and opt-in vector-location experiment were deliberately left
unchanged. The measured slide gap provided a concrete alternative witness.

The zlib-ng insertion loop contained this orphaned address chain in its store
arm:

```asm
movl %ebx, %r14d
addq %r14, %r14
leaq (%rdx, %r14), %rbp
movw %r11w, (%rdx, %rbx, 2)   # the actual address consumer
```

`%rbp` and `%r14` were never read from that chain, and the `addq` flags died
before any consumer. The existing dead-write passes already had the required
CFG-liveness and flag-liveness proofs, but admitted `%rbp` through the older
`is_relayable_family` predicate, which excludes it unconditionally. The first
LEA therefore survived and kept both feeders alive.

S84 changes only that admission in `eliminate_dead_pure_writes` and
`eliminate_dead_flag_writes`: `%rbp` is now accepted only under the existing
function-wide `RbpRoles` / `rbp_is_gpr_in_function` proof and
`is_writable_family_gpr`. Frame-pointer, CFA, pinned/inline-assembly, memory,
implicit-register, value-liveness, and flag-liveness guards are unchanged.
Role-cache invalidation follows every deletion. No benchmark-specific pattern,
new peephole pass, new environment knob, or new speculative intrinsic metadata
was added.

## Regression evidence

The new isolated regression was run against the unfixed implementation first:
the positive test failed with all three dead instructions still present. After
the admission correction:

- the isolated `dead_writes` suite passed **90/90**, including the positive
  allocator-owned-rbp chain and negative frame-pointer/CFA control;
- all selected x86 peephole tests passed **1715/1715** (5 ignored);
- the established S83 focused groups passed **214/214** with nonzero groups
  `[28, 11, 153, 19, 2, 1]`;
- `rustfmt --check`, targeted library Clippy with `-D warnings`, and
  `python3 scripts/audit_census.py --check` passed with no baseline change
  (panic 917, hot panic 101, unsafe 25, ignored tests 10, functions over
  300/500/1000 = 233/97/26, undocumented knobs 266);
- six real compile/run cells (O1, O2, Os, O2 frame pointer, O2 SSE2, O2 debug)
  preserved S83 exit status and stdout. The SSE2 cell also rejected every
  `v*` AVX/EVEX mnemonic in the disassembly. The cache has cross-target QEMU
  only, so this is explicitly a native-plus-static SSE2 gate, not a claimed
  x86-64 QEMU run;
- Memcheck completed with zero errors and identical stdout;
- the standard O2 assembly diff removes exactly the three dead loop
  instructions and the then-unneeded `%rbp` push/pop plus associated CFI rows.
  The observable store and all other hot-loop operations remain.

Raw logs and binaries are under `artifacts/S84-slide-dead-address/`.

## Object inspection

GDB disassembly of the linked candidate confirms the same three-instruction
removal in the integrated production object. Direct `objtool.real-x86
--stackval --dry-run` accepted both objects. S83's
`zng_insert_string_batch: BP used as a scratch register` warning is absent in
S84; the two pre-existing call/frame-pointer warnings remain. `pahole` emitted
no type records and is not presented as structural type evidence.

## Measured native performance

The reviewed `scripts/benchmark_native_batched.py` protocol used direct
production objects, whole-object rename identity, one neutral GCC link, fresh
child state, matched flags, stdout checks on every repetition, CPU pinning,
and 11 randomized paired rounds. Ratios below are candidate/reference elapsed
time; lower is better. Intervals are exploratory CI95, not guarantees.

### Standard flags

Flags: `-O2 -fwrapv -ffp-contract=off`, 165 iterations per batch.

| comparison | median | exploratory CI95 | interpretation |
|---|---:|---:|---|
| S84 / S83 | **0.950711241512** | **[0.911492218368, 0.970419525956]** | measured improvement over S83 |
| S84 / GCC 16.2 | **0.796865427225** | **[0.784334210699, 0.814468950599]** | lower elapsed time in this workload |
| S84 / Clang 23.1.4 | **1.065541037556** | **[1.054966293039, 1.101229864538]** | still clearly behind Clang |

### Portable ICX link configuration

Flags additionally include `-fno-builtin-memcpy -fno-builtin-memset`, 97
iterations per batch. This configuration changes the LCCC code shape and does
not contain the target chain.

| comparison | median | exploratory CI95 | interpretation |
|---|---:|---:|---|
| S84 / S83 | **1.000000000000** | n/a | byte-identical binaries and shared samples |
| S84 / GCC 16.2 | **0.895638074920** | **[0.874991776572, 0.908296910972]** | lower elapsed time in this configuration |
| S84 / Clang 23.1.4 | **1.051889170393** | **[1.030260401834, 1.068447714101]** | still behind Clang |
| S84 / ICX 2026.2 | **1.016589705898** | **[0.972866705273, 1.041299994592]** | inconclusive; no claimed ICX win |

The portable S84/S83 no-op is a useful configuration control, not evidence for
or against the standard-flag improvement.

## Callgrind cross-check

The reviewed correctness-gated `scripts/callgrind_ab.py` used standard O2 flags
and pinned cache geometry. These are simulated whole-process metrics, not PMU
cycles, uops, or measured speedups.

| S84 reference | Ir S84 | Ir reference | S84/reference |
|---|---:|---:|---:|
| S83 | 16,709,955 | 17,595,713 | **0.949660579** |
| GCC 16.2 | 16,709,955 | 22,831,700 | **0.731875200** |
| Clang 23.1.4 | 16,709,997 | 15,143,825 | **1.103419843** |
| ICX 2026.2 | 16,709,955 | 15,823,062 | **1.056050656** |

The native and simulated S84/S83 deltas agree closely. Clang remains faster in
both measurement modes; no slide victory over Clang or ICX is claimed.

## Infrastructure and boundaries

Two fixed-code build attempts stopped before running tests because the small
root filesystem filled while rustc wrote parallel incremental outputs. Only
verified duplicate/superseded fastbuild products and the interrupted builds'
specific working/temporary objects were removed. No wholesale cache, recovery,
source, or delivery deletion occurred. The successful focused test used
fastbuild O1, exactly `-j2`, `--lib`, and `CARGO_INCREMENTAL=0`; the normal
incremental driver build then completed in seconds. Details are retained in
`INFRASTRUCTURE.md`.

No `ci_local.sh --fast` or other broad local CI was run. Broader validation
remains with GitHub CI. This change closes one proven allocator-dependent
cleanup gap; it does not justify arbitrary rbp rewriting, relaxed flag
liveness, lane-extraction reclassification, or a claim that the remaining
slide/Clang gap is closed.
