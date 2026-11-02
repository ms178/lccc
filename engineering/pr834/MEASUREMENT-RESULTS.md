# Compiler-cost and stress measurements

Exact compiler source/binary provenance and the self-contained program proof
are in [CORPUS-PROVENANCE.md](CORPUS-PROVENANCE.md). Raw delivery attachments
are hash-linked by `engineering/evidence/pr834-followup-attachments.json`.
The machine was a shared two-vCPU Xeon VM, **not** isolated or a 14700KF.
All capture identity-before/after comparisons passed; no failed invocations
were removed. Compiler builds used fastbuild/O1, exactly two Cargo jobs.

## Three retained native compiler protocols

1. All45: five alternating paired batches, five invocations/arm/batch, while
   baseline stress Callgrind ran on CPU0; compiler capture on CPU1. 2,250
   compilations, zero failures. Geometric mean of median ratios: 0.9996120480;
   seven form medians exceeded 1.05.
2. Quiet all45: five batches of twenty invocations/arm, seeded form order,
   after stress completed, without concurrent builds/benchmarks/archivers.
   9,000 compilations, zero failures. Geo 0.9829413251; the earlier seven did
   not exceed 1.05, but four different forms did (table below).
3. Predeclared diagnostic: those four forms plus arith_loop/fib controls;
   five rounds, twenty pairs each, alternating first arm **within** each
   repetition. CPU time and context-switch/page-fault deltas were recorded
   separately using child rusage. 1,200 compilations, zero failures. None
   of these six median ratios exceeded 1.05. This is **not** an all45 replacement.

The earlier >5% observations remain part of the evidence. The diagnostic does
not reproduce them under closer pairing, but does not prove their cause or
justify an all45 hard timing gate. A stable native compiler regression above
5% is not established by these captures; neither is a universal <=5% bound.
Wall and CPU ratios are close in the diagnostic; it would be unsupported to
blame the earlier results specifically on scheduler waiting. No threshold
waiver, timing gate, compiler-output budget increase or target-hardware claim
is made. The native program worst15/fannkuch debt is separate and still open.

## All45 table

Ratios are candidate / baseline; wall columns are ratios of arm medians, each over five
batch means, not simulated counters. Compiler Ir is one separate default-size
parent-process profile per arm; it excludes exec children. Native walltime
includes assembler/linker work. Program Ir uses the separate bounded corpus.

| Form | Initial wall | Quiet wall | Compiler Ir | Diagnostic wall / CPU |
|---|---:|---:|---:|---:|
| arith_loop | 1.088085 | 0.957964 | 0.999557 | 0.989221 / 0.989154 |
| fib | 1.061666 | 1.010799 | 1.001602 | 0.993802 / 0.994103 |
| matmul | 0.960442 | 0.994349 | 1.000518 | — |
| qsort | 0.988213 | 1.001987 | 1.000759 | — |
| sieve | 1.015741 | 1.027703 | 1.000294 | — |
| tce_sum | 0.959852 | 0.978693 | 0.998123 | — |
| nbody | 0.961808 | 0.992677 | 1.001809 | — |
| binary_trees | 1.010820 | 0.944837 | 1.000164 | — |
| spectral_norm | 0.994046 | 0.952617 | 1.001825 | — |
| mandelbrot | 0.925658 | 1.078154 | 1.000256 | 0.986877 / 0.987566 |
| hash_table | 0.976872 | 1.056568 | 1.000112 | 0.999284 / 1.000228 |
| strlen_bench | 1.000844 | 1.059296 | 1.000494 | 0.979666 / 0.979736 |
| switch_dispatch | 1.007302 | 1.015949 | 0.999693 | — |
| struct_copy | 1.020396 | 0.917375 | 1.000517 | — |
| loop_patterns | 0.978485 | 1.018830 | 1.001719 | — |
| fannkuch | 1.031731 | 0.949079 | 1.001829 | — |
| ackermann | 0.869583 | 0.985897 | 1.006314 | — |
| constant_recursion | 1.145922 | 0.961970 | 1.005313 | — |
| bitops | 1.007382 | 0.995344 | 1.001443 | — |
| double_reduction | 0.992956 | 0.933832 | 1.003098 | — |
| ascii_case_fold | 0.993857 | 1.009068 | 1.001351 | — |
| binary_search | 0.983799 | 1.025084 | 1.000200 | — |
| ring_fifo | 1.008638 | 0.942913 | 1.004297 | — |
| aarch64_select_patterns | 1.009136 | 0.946921 | 0.997762 | — |
| histogram | 0.996485 | 0.990844 | 1.000044 | — |
| gzip_crc32 | 1.032970 | 0.964677 | 1.001293 | — |
| libm_round_family | 1.034862 | 0.992759 | 1.001931 | — |
| tls_seg_access | 1.048178 | 1.155045 | 1.000783 | 1.013189 / 1.012974 |
| percpu_ptr_walk | 0.914739 | 0.975595 | 0.998594 | — |
| byte_load_reassembly | 1.063605 | 0.942746 | 0.999855 | — |
| zlib_ng_adler32 | 0.998868 | 0.996217 | 1.000356 | — |
| zlib_ng_adler32_combine | 0.991347 | 0.893980 | 0.999717 | — |
| expat_xml_scan | 1.007283 | 0.975471 | 1.000114 | — |
| expat_siphash24 | 1.052828 | 0.993475 | 1.000594 | — |
| sqlite_varint | 0.998286 | 0.975565 | 0.997825 | — |
| linux_find_bit | 0.918434 | 0.910846 | 1.000971 | — |
| linux_find_bit_scaled | 1.049378 | 1.010421 | 1.000971 | — |
| glibc_memcmp | 0.984370 | 0.943504 | 1.001603 | — |
| chacha20_block | 1.004936 | 0.995201 | 1.000286 | — |
| sha256_transform | 1.053754 | 0.992480 | 1.000060 | — |
| linux_rbtree | 0.939677 | 0.990896 | 1.000237 | — |
| zstd_count | 1.056632 | 0.953291 | 0.999991 | — |
| lz4_compress | 0.980577 | 0.932764 | 1.001418 | — |
| lz4_match_extend | 0.950900 | 1.015055 | 1.001418 | — |
| glibc_strstr | 0.964034 | 0.922505 | 1.000210 | — |

Compiler-parent Ir geo: **1.0008280867352997**, range
**[0.9977620997240416, 1.0063138232679372]**. This does not explain away
walltime outliers. All 225 bounded program Ir ratios are exactly 1, and native
and instrumented outputs agree. 44/45 ELFs agree between main and candidate;
all 225 full A/B binary-hash records match the earlier F1/F6 candidate capture.
Thus F2 introduced no additional program-byte change in these captured cases.

## 10,000-instruction narrow-copy stress

The manual ignored test was explicitly executed, not silently skipped. Both
arms assert all 2,500 forwardings. Full refreshes fell from 2,500 to zero.
Five alternating native **pass** timings have medians 17.580829101 s and
0.011672458 s, ratio 0.0006639310315197859; every sample, including the
candidate outlier, is retained. This is a synthetic pass result, not a
whole-compiler or generated-program speedup.

Both complete Callgrind runs exited successfully. Baseline Ir:
**286,143,207,685**; candidate Ir: **258,558,917**; ratio
**0.0009035997013237998**. These whole-test-process counters include harness
overhead. Full cache/branch counters and original profiles are retained.
The baseline instrumented test took 4,887.89 s while other work overlapped;
that elapsed time is explicitly **not** a native timing result. It was never
terminated, reset or replaced by an extrapolated partial counter.

The optimization is a read-rename cache, not delayed recomputation after
arbitrary edits: propagation itself does not delete the copy or change writes.
Other families retain eager refresh. Added private-register reads invalidate
queries; a negative stale answer after removed reads refreshes and retries.
Two invalidation tests and 64 eager/lazy exact-output CFG comparisons guard it.

## Remaining proof boundaries

- Local FAST/format/strict-Clippy receipts and hosted CI are separate from
  measurement results; the final delivery status states what actually ran.
- Hosted post-change duration/failure/flake observations are still 0/20.
- Wide-vector frontend interoperability and return ABI are not fully proved.
- Conservative xrstor handling, additional effect-table coverage and inherited
  assembly-listing/native-AVX2 portability limitations are not claimed solved.
- Some inherited move comments are too broad: legacy XMM-to-XMM MOVQ zeroes
  bits 127:64, and three-operand scalar VEX moves merge part of low128 rather
  than zeroing everything above the scalar element. No instruction-admission
  expansion was justified by those comments; their cleanup remains separate.

## Local acceptance provisioning finding

The first immutable FAST attempt on the publication tree produced
246 PASS / 7 FAIL / 6 SKIP. All seven failed mandatory i386 legs because the
relocated GCC16 install lacked its own 32-bit `libgcc.a`; the passed m64 legs
were not counted as complete gates. Extracting the matching
`lib32gcc-16-dev_16.2.0-3_amd64.deb` restored the executable -m32 probe and all
seven targeted gates, without compiler/test/gate changes. The package URL,
SHA-256 and exact extracted archive identity are linked by the attachment
manifest. Host shared libgcc_s remains 14.2.0-19; this is not described as a
complete GCC16 multilib package installation. Native64 measurement conditions
were not retroactively relabelled.

The first MSRV leg also explicitly auto-installed pinned Rust1.97.0; its
rustup provisioning warnings are retained, distinct from compiler/Clippy
warnings. Final whole-FAST status is recorded only after the fresh run.

## Later compiler lifecycle repair: scope boundary

The delivery subsequently repairs `src/common/temp_files.rs`. The all45 and
stress measurements above remain attributed to b52a114, not this later compiler.
Historical verification remains valid; `--check-current-inputs` must reject the
changed compiler input rather than silently blessing it. The exact-current-tree
remeasurement and oracle/runtime work are explicitly queued in `backlog.md`.
