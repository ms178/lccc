# S86 compiler measurements and follow-through — 2026-10-15

## Verdict and identity

**Performance acceptance withheld.** No broad win over main or GCC16.2 was
established. The S86 compiler SHA256 is
`aa0eca9aa5a56b26e06a850ecf2872bb9c25df67c985fb68ac6f6f5f0f0389ed`;
main-f07 is `d1d0b37c2cbc0354c602201827c89834598a907f24df14b4e054bf39960ecd68`.
These measurements belong to those immutable binaries, not automatically to
any later compiler. S87/S88 changed tooling/evidence only. Full raw summaries
and content hashes are in `evidence/redteam88-measurements/`.

Ratios are candidate/reference elapsed time: **lower is better**. Native45 uses
nine randomized paired rounds, two warmups, CPU0 affinity, identical target
flags per workload and no removed outliers. There are two allowed virtual CPUs,
no accessible governor/frequency control, and no physical EPYC/Raptor Lake
claim. Values below the 20ms all-arm median floor are separately identified.
Bootstrap intervals describe these samples, not independent physical hosts.
The 31-pair follow-up is a selected investigation, never a replacement of the
initial 45-case aggregate. Small evidence writes/review commands occurred during
follow-up; no simultaneous compiler build or second benchmark was scheduled.

## PMU metadata correction

The original native45 record incorrectly says PMU available. A real
`perf stat --no-big-num -x, -e cycles,instructions,branches,branch-misses -- /bin/true`
returned zero while every event was `<not supported>`. S87 now requires positive,
finite, unambiguous cycles AND instructions counts and preserves command/stderr.
Six PMU and five native lifecycle tests pass (11, not the 12 mistakenly written
in the checkpoint subject). The corrected real probe is unavailable with empty
observed counts. Raw historical metadata is preserved, not silently edited.
Wall timings do not depend on these counters. Callgrind below is simulation.

## Native45 aggregates

| Population | Reference | n | Geomean | Arithmetic mean | Best | Worst |
|---|---|---:|---:|---:|---:|---:|
| All (includes launch-floor cases) | main | 45 | 1.022173 | 1.024409 | 0.918557 | 1.335254 |
| All (includes launch-floor cases) | gcc | 45 | 0.783500 | 1.016791 | 0.008894 | 1.877449 |
| All arms median ≥20ms | main | 28 | 1.011297 | 1.012337 | 0.963963 | 1.255263 |
| All arms median ≥20ms | gcc | 28 | 1.057436 | 1.069925 | 0.642627 | 1.465913 |

### Every native shape, and corresponding bounded Callgrind Ir

The large default matmul/constant-recursion ratios are below the wall-time
floor, not accepted regressions or wins. The qualified fannkuch regression is
real: +25.53%, tight interval, with previously executed no-wrap ablations
identifying the cost of wrapping-safe proof restrictions. Removing `-fwrapv`
semantics is rejected. CG fannkuch Ir is 1,262,312 versus 1,230,058; equal simulated
D1/I1/LL miss totals do not establish equal runtime or recover physical PMU data.
No candidate/main whole native ELF is identical (0/45), so timing differences
cannot be dismissed on a claim of identical whole binaries.

| Shape | ≥20ms | /main | 95% interval | /GCC16 | CG Ir /main |
|---|:---:|---:|---|---:|---:|
| arith_loop | yes | 1.005492 | [0.973199, 1.018911] | 1.011248 | 1.000000 |
| fib | no | 1.017236 | [0.823361, 1.081586] | 0.008894 | 1.000000 |
| matmul | no | 1.335254 | [1.268356, 1.361487] | 1.356052 | 1.001391 |
| qsort | yes | 1.005108 | [0.989947, 1.028174] | 1.004434 | 1.000000 |
| sieve | yes | 1.009520 | [0.973064, 1.083050] | 1.081999 | 1.118732 |
| tce_sum | no | 1.075643 | [0.958427, 1.120902] | 1.103638 | 1.000000 |
| nbody | yes | 1.014817 | [0.940093, 1.154237] | 1.056936 | 0.987080 |
| binary_trees | yes | 1.018165 | [0.956534, 1.037004] | 1.191529 | 1.000000 |
| spectral_norm | yes | 1.026378 | [0.881622, 1.067039] | 1.418821 | 0.971018 |
| mandelbrot | yes | 1.002423 | [0.973364, 1.041588] | 1.197854 | 1.000000 |
| hash_table | yes | 0.963963 | [0.931163, 1.029949] | 0.990690 | 1.000000 |
| strlen_bench | yes | 1.025202 | [0.900983, 1.120323] | 0.963010 | 1.009298 |
| switch_dispatch | yes | 1.001059 | [0.995054, 1.034862] | 1.201117 | 1.000000 |
| struct_copy | yes | 0.997668 | [0.897115, 1.007207] | 0.906356 | 1.000000 |
| loop_patterns | yes | 0.985943 | [0.957944, 0.991045] | 1.152402 | 1.000010 |
| fannkuch | yes | 1.255263 | [1.230693, 1.261970] | 1.465913 | 1.026222 |
| ackermann | no | 1.041775 | [0.861934, 1.192132] | 0.009048 | 1.000000 |
| constant_recursion | no | 1.233208 | [0.958274, 1.351605] | 0.010514 | 1.000000 |
| bitops | yes | 0.987897 | [0.960701, 1.002628] | 0.642627 | 1.000000 |
| double_reduction | yes | 0.999245 | [0.994582, 1.008113] | 1.002726 | 1.000000 |
| ascii_case_fold | no | 1.003736 | [0.934337, 1.041698] | 1.046460 | 1.000000 |
| binary_search | no | 1.011425 | [0.947918, 1.033080] | 0.929121 | 1.002929 |
| ring_fifo | no | 1.017403 | [0.892943, 1.078912] | 1.024546 | 1.000000 |
| aarch64_select_patterns | yes | 0.999962 | [0.985443, 1.029957] | 1.034340 | 1.000000 |
| histogram | no | 1.002907 | [0.886095, 1.068973] | 1.087609 | 1.000000 |
| gzip_crc32 | yes | 0.997882 | [0.992279, 1.001523] | 0.873067 | 1.000000 |
| libm_round_family | yes | 1.002938 | [0.995906, 1.007354] | 1.094371 | 1.000000 |
| tls_seg_access | no | 1.002078 | [0.978125, 1.070612] | 1.018436 | 1.000000 |
| percpu_ptr_walk | no | 1.000470 | [0.991607, 1.018803] | 1.003734 | 1.000000 |
| byte_load_reassembly | no | 0.918557 | [0.862012, 0.932964] | 1.877449 | 0.875018 |
| zlib_ng_adler32 | yes | 0.990225 | [0.951510, 1.009654] | 0.999903 | 1.000000 |
| zlib_ng_adler32_combine | yes | 0.999666 | [0.957355, 1.042790] | 1.012148 | 1.000000 |
| expat_xml_scan | yes | 1.040168 | [1.012237, 1.044633] | 1.213808 | 0.990013 |
| expat_siphash24 | yes | 0.984731 | [0.870556, 1.061748] | 0.971782 | 1.000000 |
| sqlite_varint | yes | 1.014749 | [0.921414, 1.138148] | 1.196084 | 1.021882 |
| linux_find_bit | no | 1.009468 | [0.787860, 1.065322] | 1.177706 | 1.000000 |
| linux_find_bit_scaled | yes | 1.002516 | [0.954034, 1.051442] | 1.202780 | 1.000000 |
| glibc_memcmp | no | 1.005518 | [0.975987, 1.030146] | 1.009691 | 1.000000 |
| chacha20_block | yes | 0.998007 | [0.914138, 1.021910] | 0.883196 | 1.000000 |
| sha256_transform | yes | 1.009252 | [0.960714, 1.026429] | 1.110744 | 1.006749 |
| linux_rbtree | no | 0.995879 | [0.978901, 0.998332] | 1.018431 | 1.000000 |
| zstd_count | no | 0.998276 | [0.978311, 1.006381] | 1.045405 | 1.004454 |
| lz4_compress | no | 1.084114 | [0.973554, 1.151693] | 1.070953 | 1.007796 |
| lz4_match_extend | yes | 1.003564 | [0.961027, 1.011004] | 1.009792 | 0.999998 |
| glibc_strstr | yes | 1.003636 | [0.998463, 1.015056] | 1.068232 | 1.036176 |

Callgrind45: Ir GM 1.000930, AM 1.001306. Explicit cache geometry was
I1/D1 32KiB, 8-way; LL 32MiB, 16-way; 64-byte lines. Raw headers confirm
these values despite startup L3 warnings. Counts are bounded whole-process
simulation, not speedup predictions. All 45 outputs agreed.

## Patched zstd: 96 API cases

Pinned archpkgbuilds recipe commit `655b50b99531f7d86182275a1ccf3460e5b9fe2b`,
zstd1.5.7/pkgrel16, source SHA256
`5b331d961d6989dc21bb03397fc7a2a4d86bc65a14adc5ffbbce050354e30fd2`, patch
`aa15486d10778c9d00a9b6273101e11feb952aca0b66432a53625dfb67390d8e`.
Six 1MiB corpora, eight profiles, compression/decompression: 96 cases, nine
paired rounds, shared GCC driver with separately compiled codec libraries.
Each operation is checked outside timing and frames cross-decoded with GCC CLI.
This is NOT the recipe's LLVM24 Full-LTO/PGO/BOLT production build. Threaded
profiles use both available virtual CPUs; others use the first. Actual native
API operation times, not fixed-duration CLI `-b` wall time, are compared.

| Operation | Reference | n | Geomean | Arithmetic mean | Best | Worst |
|---|---|---:|---:|---:|---:|---:|
| Compress | main | 48 | 0.997887 | 0.998073 | 0.934474 | 1.071689 |
| Compress | gcc | 48 | 1.445530 | 1.463961 | 0.972582 | 2.090510 |
| Decompress | main | 48 | 1.007757 | 1.008071 | 0.964338 | 1.113461 |
| Decompress | gcc | 48 | 1.307746 | 1.324858 | 0.998039 | 1.791879 |

### Every profile (C = compression; D = decompression)

| Corpus/profile | C /main | C /GCC | D /main | D /GCC |
|---|---:|---:|---:|---:|
| source-fast5 | 1.007246 | 1.517029 | 0.998521 | 1.427316 |
| source-level1 | 0.971576 | 1.504081 | 1.067122 | 1.506231 |
| source-level3 | 1.022930 | 1.547685 | 0.989189 | 1.474946 |
| source-level9 | 1.008207 | 1.648811 | 1.076082 | 1.412285 |
| source-level19 | 1.000291 | 1.530914 | 1.113461 | 1.581089 |
| source-long20 | 1.023817 | 1.590126 | 1.010283 | 1.498910 |
| source-threads2 | 0.999991 | 1.463198 | 1.024633 | 1.518658 |
| source-no_checksum | 0.986967 | 1.517042 | 1.017255 | 1.561341 |
| json-fast5 | 0.975405 | 1.725581 | 0.997053 | 1.517431 |
| json-level1 | 0.989645 | 1.536547 | 0.984126 | 1.457329 |
| json-level3 | 0.992770 | 1.530054 | 1.043570 | 1.574063 |
| json-level9 | 1.002392 | 1.672904 | 1.019686 | 1.438244 |
| json-level19 | 0.996339 | 1.462234 | 0.966750 | 1.411059 |
| json-long20 | 0.988040 | 1.564104 | 1.019286 | 1.604908 |
| json-threads2 | 0.981499 | 1.468952 | 1.023800 | 1.583441 |
| json-no_checksum | 0.999076 | 1.556197 | 1.023189 | 1.659764 |
| random-fast5 | 0.997533 | 1.344561 | 0.990370 | 1.010048 |
| random-level1 | 0.999080 | 1.179857 | 1.002451 | 0.998039 |
| random-level3 | 1.017658 | 1.296060 | 0.996948 | 1.001202 |
| random-level9 | 0.994665 | 1.445802 | 0.998000 | 1.014623 |
| random-level19 | 0.934474 | 2.063629 | 1.002552 | 1.007520 |
| random-long20 | 0.989098 | 1.371497 | 0.984099 | 1.007450 |
| random-threads2 | 1.071689 | 1.200623 | 1.010892 | 1.017629 |
| random-no_checksum | 0.996082 | 1.357216 | 1.006050 | 1.003077 |
| rle-fast5 | 1.021084 | 1.130293 | 1.000271 | 1.099106 |
| rle-level1 | 1.001618 | 1.130408 | 0.997065 | 1.098927 |
| rle-level3 | 1.009785 | 1.290667 | 0.974817 | 1.104640 |
| rle-level9 | 0.975092 | 0.972582 | 0.988869 | 1.096376 |
| rle-level19 | 0.998586 | 1.006439 | 0.990235 | 1.090043 |
| rle-long20 | 0.971029 | 1.245362 | 1.003549 | 1.129005 |
| rle-threads2 | 0.977524 | 1.145000 | 1.015182 | 1.114882 |
| rle-no_checksum | 0.997092 | 1.392201 | 1.003114 | 1.262572 |
| duplicates-fast5 | 1.002104 | 1.272584 | 1.005808 | 1.413937 |
| duplicates-level1 | 0.968962 | 1.243929 | 0.997109 | 1.397957 |
| duplicates-level3 | 0.989494 | 1.402882 | 0.997727 | 1.396037 |
| duplicates-level9 | 1.003409 | 1.472164 | 1.009888 | 1.430631 |
| duplicates-level19 | 1.000194 | 1.498529 | 0.996047 | 1.402470 |
| duplicates-long20 | 0.996059 | 1.412183 | 1.002252 | 1.398228 |
| duplicates-threads2 | 1.023311 | 1.232173 | 0.990747 | 1.433424 |
| duplicates-no_checksum | 0.998910 | 1.488465 | 1.010649 | 1.791879 |
| elf-fast5 | 1.001311 | 1.804902 | 0.964338 | 1.310304 |
| elf-level1 | 0.999482 | 1.768914 | 0.992337 | 1.210519 |
| elf-level3 | 0.992569 | 1.656637 | 1.015022 | 1.329244 |
| elf-level9 | 1.013495 | 2.090510 | 1.034494 | 1.361759 |
| elf-level19 | 1.010030 | 1.448382 | 1.012317 | 1.389008 |
| elf-long20 | 0.997404 | 1.763066 | 1.007000 | 1.383417 |
| elf-threads2 | 1.013243 | 1.605927 | 1.005582 | 1.327457 |
| elf-no_checksum | 0.999261 | 1.701229 | 1.007628 | 1.334743 |

## RLE and correctness follow-through

The 31-pair RLE-level1 experiment used the EXACT original S77/S79/GCC driver
hashes. Old S79/S77 compression is 1.008417 [0.981142,1.014376]; candidate/S77
1.007452 [0.988444,1.023340]. The historical +8.14% did not reproduce. This is
not evidence that a new compiler change fixed RLE. Both experiments are retained.

After the S86 frontend repairs: 4,806 library tests pass, eight ignored;
16 native Memcheck fixtures, 12 cross-target executions and eight CE compiler
executions pass; fresh CLI matrix 1,976 commands/zero failures; six shared-driver
upstream fuzz/zstream runs of 500 iterations each pass. Historical S85 CLI
3,209 commands/nine failures remains a failed record. Main-f07's known-bad LZ4
control is not counted as a success. Candidate/GCC/main fresh build logs had
zero warnings; that does not describe every earlier build in this project.

## Architecture decisions and remaining costs

- Agree with PR834's region-aware vector copies, final VEX upper-liveness,
  duplicate-SSA rejection, CFG assertions and evidence/tested-build gates.
  Integration preserved those safeguards. Passing examples alone are not proof.
- Reject the historical raw late-fuser deletion of live Boolean values, flags,
  and frame bytes. One shared CFG/pin/full-width legality authority replaces
  the duplicate implementation; successful rewrites refresh affected facts.
- Mixed RBP/RSP slot coordinates remain opaque absent a proved frame delta.
  This is conservative and may cost optimization opportunities. A CFG-aware
  frame-coordinate proof is preferable to unsound base-register disjointness.
- Retain strict FP and modular-integer semantics. No benchmark-name dispatch,
  no-wrap assumption, relaxed alias rule or false baseline waiver is accepted.
- GCC16.2.0-3 is the installed/candidate Debian package; Rust/cargo1.99.0,
  rustfmt1.10.0 and Clippy0.1.99 were checked. 6GiB swap is active. Compiler
  builds use fastbuild O1/j2. Only FAST CI is authorized locally.
- Godbolt's `icxlatest` actually executed2024.0.0, not verified vendor-latest
  2026. ICC optimized counterexamples with passing O0 controls remain failures,
  not references to copy. No physical-host or external-expert approval exists.

See `backlog.md` for concrete proof/profitability and tooling work. The current
receipt, not this historical measurement record, determines FAST acceptance of
the final source tree. A green correctness gate is not performance approval.

## Selected 31-pair follow-up (completed after the upstream check)

The earlier interrupted native report remains `complete=false`. Its nbody row
had all 31 pairs; binary_trees was interrupted before a completed row. The
remaining five cases were rerun in a separate complete report. No partial
binary_trees samples are counted. All compilers retained their S86/main-f07/GCC
hashes. These are selected-case diagnostics, not a new 45-case acceptance run.

| Native case | /main | 95% bootstrap interval | /GCC |
|---|---:|---|---:|
| nbody | 1.029854 | [1.009685552683509, 1.0436851766976483] | 0.995458 |
| binary_trees | 0.998280 | [0.98386767937653, 1.013968115005947] | 1.165146 |
| spectral_norm | 0.987975 | [0.9851551159603912, 0.9924045268237061] | 1.406268 |
| strlen_bench | 0.999091 | [0.9896976704055983, 1.0081853523981472] | 1.028843 |
| expat_xml_scan | 1.043847 | [1.0363684145327394, 1.0493702842268386] | 1.211825 |
| sqlite_varint | 1.009103 | [0.9914667146960281, 1.0233765694773624] | 1.226641 |

XML's +4.38% is confirmed; nbody's completed 31-pair row is +2.99%.
Binary trees, strlen and varint intervals span parity; spectral_norm reverses
the initial noisy sign. This does not excuse confirmed regressions or prove
that unmeasured code changed. Existing XML affine-fold ablation (~3.52% cost)
requires a general profitability/layout investigation, not a workload exception.

The 13 prespecified zstd profiles completed 31 pairs in both operations (26
cases), zero failures. Results below stay separate from the original zstd96.

| Profile | C /main [95% interval] | D /main [95% interval] |
|---|---|---|
| source-level1 | 0.994571 [0.985752, 1.001721] | 1.006808 [0.979745, 1.012504] |
| source-level3 | 0.999901 [0.986868, 1.019306] | 1.012704 [0.993112, 1.017695] |
| source-level9 | 1.003997 [0.992358, 1.023986] | 1.005015 [0.990787, 1.009308] |
| source-level19 | 1.005293 [0.987184, 1.031931] | 1.014195 [1.004860, 1.025406] |
| source-long20 | 0.988996 [0.973619, 0.996200] | 1.010125 [0.970204, 1.031751] |
| source-threads2 | 0.996099 [0.939183, 1.026349] | 1.032890 [1.016064, 1.063963] |
| random-threads2 | 0.994218 [0.971854, 1.057288] | 1.011861 [0.995262, 1.029928] |
| json-level3 | 1.010173 [0.998805, 1.020413] | 1.003171 [0.998582, 1.014026] |
| json-threads2 | 1.007402 [0.973121, 1.072490] | 1.019423 [0.972265, 1.054251] |
| json-no_checksum | 1.013705 [1.003607, 1.025798] | 1.019530 [1.004892, 1.021589] |
| elf-level9 | 1.007207 [0.944682, 1.079286] | 0.998892 [0.991239, 1.012064] |
| rle-fast5 | 0.979977 [0.950229, 0.998982] | 0.998893 [0.993813, 1.011864] |
| duplicates-threads2 | 0.987678 [0.975661, 1.043304] | 1.007009 [0.999615, 1.022377] |

All 15 ELFs in the five-case native follow-up are byte-identical to their
already archived native45 counterparts (three compiler arms per case); the
identity manifest records both hashes. No rebuild is substituted for measured
executables. Experiment drivers are retained in the evidence `tools/` directory;
they take explicit compiler/repository/source/output arguments. The raw JSON
contains actual flags, source hashes, paired order, measurements and commands.

## Applicability to the literal-payload repair

The later compiler `0f555f740170041c63af9c06d0080e56a37a146b37845a6ad1d83728c9cd2ba7`
fixes additional frontend correctness defects. Direct recompilation proves
all45 native ELFs and all49 zstd compiler-produced objects unchanged; relinked zstd static
library, CLI and API driver also match S86 byte-for-byte. The old timing data
therefore describe identical generated artifacts, not a claimed new timing run.
See `LITERAL-PAYLOAD-REDTEAM-2026-10-15.md` for validation and identity records.
