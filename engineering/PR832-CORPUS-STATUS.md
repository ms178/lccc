# PR832: all-45 performance disposition

Current main 63e783b; candidate compiler source 75b74f1. All 45 outputs match.
Three alternating equal-path pairs per workload (270 instrumented runs): every
candidate/main Ir range is **[1.000000, 1.000000]**. 44/45 ELFs are byte-identical;
libm_round_family differs only in cold helper instructions, not its hot loop.
All supplied worst15 ELFs are identical. This hardening is NOT a runtime speedup.

GCC16 whole-program bounded Ir geomean: 0.988403. Startup/library work is included.
Pinned sizes: tests/benchmark/callgrind_sizes.json. Flags: -O2 -march=x86-64-v3
-fwrapv -Wl,--no-as-needed -lm, plus those size macros. Fixed I1/D1/LL geometry;
GCC16.2 builtin headers; final assembler/bfd 2.47, lld23.1.4, mold2.42.1.
The initial diagnostic sweep used host binutils2.44/mold2.37.1; it remains
historical and is NOT relabelled as the pinned final run. The compiler binaries
were independently built O1/-j2. Main predates the final linker provisioning;
the final candidate was built with the pinned tools. Their
source and binary hashes, exact tools, argv and raw streams are in the delivery.

## Every registered form

| Form | main/GCC simulated Ir | candidate/main, 3 pairs | Same ELF |
|---|---:|---:|:---:|
| aarch64_select_patterns | 1.213691 | 1.000000–1.000000 | yes |
| ackermann | 0.126694 | 1.000000–1.000000 | yes |
| arith_loop | 0.823918 | 1.000000–1.000000 | yes |
| ascii_case_fold | 1.621353 | 1.000000–1.000000 | yes |
| binary_search | 1.396830 | 1.000000–1.000000 | yes |
| binary_trees | 1.126595 | 1.000000–1.000000 | yes |
| bitops | 0.730681 | 1.000000–1.000000 | yes |
| byte_load_reassembly | 4.899262 | 1.000000–1.000000 | yes |
| chacha20_block | 0.509346 | 1.000000–1.000000 | yes |
| constant_recursion | 0.126499 | 1.000000–1.000000 | yes |
| double_reduction | 1.125835 | 1.000000–1.000000 | yes |
| expat_siphash24 | 0.990143 | 1.000000–1.000000 | yes |
| expat_xml_scan | 1.161733 | 1.000000–1.000000 | yes |
| fannkuch | 1.119859 | 1.000000–1.000000 | yes |
| fib | 0.072956 | 1.000000–1.000000 | yes |
| glibc_memcmp | 1.044518 | 1.000000–1.000000 | yes |
| glibc_strstr | 1.771742 | 1.000000–1.000000 | yes |
| gzip_crc32 | 0.991036 | 1.000000–1.000000 | yes |
| hash_table | 1.042268 | 1.000000–1.000000 | yes |
| histogram | 1.905258 | 1.000000–1.000000 | yes |
| libm_round_family | 0.972668 | 1.000000–1.000000 | no |
| linux_find_bit | 1.173396 | 1.000000–1.000000 | yes |
| linux_find_bit_scaled | 1.182394 | 1.000000–1.000000 | yes |
| linux_rbtree | 1.253698 | 1.000000–1.000000 | yes |
| loop_patterns | 1.250029 | 1.000000–1.000000 | yes |
| lz4_compress | 1.120054 | 1.000000–1.000000 | yes |
| lz4_match_extend | 1.242105 | 1.000000–1.000000 | yes |
| mandelbrot | 1.088427 | 1.000000–1.000000 | yes |
| matmul | 0.901388 | 1.000000–1.000000 | yes |
| nbody | 1.302978 | 1.000000–1.000000 | yes |
| percpu_ptr_walk | 0.983851 | 1.000000–1.000000 | yes |
| qsort | 1.034448 | 1.000000–1.000000 | yes |
| ring_fifo | 2.484617 | 1.000000–1.000000 | yes |
| sha256_transform | 0.942746 | 1.000000–1.000000 | yes |
| sieve | 0.942040 | 1.000000–1.000000 | yes |
| spectral_norm | 2.080722 | 1.000000–1.000000 | yes |
| sqlite_varint | 1.066897 | 1.000000–1.000000 | yes |
| strlen_bench | 1.128115 | 1.000000–1.000000 | yes |
| struct_copy | 0.937840 | 1.000000–1.000000 | yes |
| switch_dispatch | 1.080908 | 1.000000–1.000000 | yes |
| tce_sum | 0.952358 | 1.000000–1.000000 | yes |
| tls_seg_access | 0.971399 | 1.000000–1.000000 | yes |
| zlib_ng_adler32 | 0.918000 | 1.000000–1.000000 | yes |
| zlib_ng_adler32_combine | 1.022448 | 1.000000–1.000000 | yes |
| zstd_count | 1.105875 | 1.000000–1.000000 | yes |

## Worst15: open optimization work, not a claim of fifteen fixes

The user-provided EPYC/KVM wall-time ratios are separate evidence with different
revision/input/measurement conditions. They cannot be replaced by this Ir rank.
Every item below has retained compiler-arm disassembly and Callgrind profiles.
Hypotheses below are next investigations, not proven native bottlenecks.

| Form | supplied wall ratio | bounded main/GCC Ir | Observed shape / next proof needed |
|---|---:|---:|---|
| chacha20_block | 2.0116 | 0.509346 | 4-lane XMM ARX already present; core Ir below GCC. Measure dependency latency/throughput on target hardware, not just instruction count. |
| byte_load_reassembly | 1.7987 | 4.899262 | Hot LE32 is ALREADY one load. LCCC keeps scalar nested reduction; GCC vectorizes repeated rounds. Need a general stride/reduction legality proof, not another byte-load peephole. |
| spectral_norm | 1.4297 | 2.080722 | Packed strict-FP prefix plus scalar tail exists. Compare denominator arithmetic, ordered accumulation and register traffic; no FP reassociation permission. |
| sieve | 1.2591 | 0.942040 | Lower Ir than GCC, similar simulated store misses. Native memory/branch behavior remains unproven. |
| sha256_transform | 1.2344 | 0.942746 | Core Ir is slightly lower, but load/store mix differs. Schedule/register-pressure diagnosis needs native target data. |
| binary_trees | 1.1989 | 1.126595 | Recursive make/check and allocator work both matter. Investigate recursion/inlining without hiding allocation correctness. |
| expat_xml_scan | 1.1709 | 1.161733 | Scanner function Ir is already slightly below GCC; main setup/control contributes to whole-program gap. Do not optimize only the named scanner. |
| lz4_match_extend | 1.1369 | 1.242105 | Inlined main has substantially more simulated data accesses. Separate dictionary accesses, reloads and match-control paths before changing alias/RA rules. |
| sqlite_varint | 1.0792 | 1.066897 | Decoder Ir is below GCC; encoder/main are higher. Preserve all varint widths and malformed/truncated-input checks. |
| fannkuch | 1.0548 | 1.119859 | Current candidate is byte-identical to main. GCC gap remains; the rejected historical +4.2758% waiver is NOT reinstated. |
| hash_table | 1.0532 | 1.042268 | Pointer-chasing plus allocation; both insert/lookup use more instructions. Do not infer native speed from bounded whole-program Ir. |
| tls_seg_access | 1.0518 | 0.971399 | Whole-program Ir below GCC while tls_pass itself is slightly above. Startup dominates this small input; preserve segment/address-space semantics. |
| mandelbrot | 1.0427 | 1.088427 | Register-only scalar FP hot loop; instruction count above GCC. Strict rounding/contraction behavior remains a hard constraint. |
| strlen_bench | 1.0378 | 1.128115 | String-building/main work contributes materially, not just libc strlen. Separate setup from scanned-byte work at default sizes. |
| linux_rbtree | 1.0362 | 1.253698 | Main contains more instructions/data traffic; tree rotations, color packing and lookup paths require separate general proofs. |

**Not completed:** eliminating all15 GCC wall-time deficits; Raptor Lake/14700KF
native/PMU validation; a fresh all45 Clang23.1/ICX execution matrix; hosted CI.
No optimization family is disabled to manufacture a favorable score.
