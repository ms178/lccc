# PR831 comparison — measured findings

Latest oracle/acceptance follow-through: [PR831-ORACLE-RECHECK.md](PR831-ORACLE-RECHECK.md).
The post-unit-repair compiler is `bca00af65791e99ac8f1cbc56ce6a5d49a99965540810715314cd8846c76be49`;
90 complete benchmark ELFs and seven Haswell assembly listings remain identical.
Older compiler hashes and flag-specific results below retain their historical identities.

**Performance approval is withheld.** The candidate repairs concrete wrong-code/linker defects and recovers the TLS instruction-count loss, but the expanded native screen is not an overall win and fannkuch retains a substantial correctness-related regression. A green fast gate is a correctness/build gate, not performance approval.


## Latest-main refresh: PR832 and revised PR831

**Current base:** `63e783b15586eca86f306bd75a52b82c57304bbb`; **reviewed PR831:** `acd48b0ca8b8427e95d30ecdae72bfb1fd888f38`.
Current candidate compiler: `3b78f2e710e8c0d7fd3949a005623d3abcbe3455fbba085caeb44df2b9f40656`.
Latest-main control: `34d37df9c6d7e805acff976ab5798d36bae53bc1d99f58005326cc7e3cdff8aa`.
Refreshed PR831 control: `a7ed15ca95057f8e8ca6723e80149ee244e3190cabfe9c3e93e7ad1d3abdf4ce`.

The prior results below retain their original identities. Both the final candidate and refreshed PR831 each reproduce **90 whole benchmark executables byte-for-byte** from their earlier measured counterparts (45 default native + 45 bounded). The debug-free fastbuild compiler is separately verified. New control builds are independent and serial; controls share a Cargo cache with each other, not the candidate. O1/j2 and active 6 GiB swap are retained.

### Fresh native comparison with latest main

20 workloads, 3 arms, nine paired randomized rounds, two warmups, CPU0, all outputs equal. Native measurements ran without builds/CI/instrumentation concurrently. VM proxy limitations still apply; no physical EPYC/Raptor Lake or PMU claim.

| Set | Reference | GM | AM |
|---|---|---:|---:|
| 20 workloads | main63 | 1.009473 | 1.011519 |
| 20 workloads | gcc | 1.166669 | 1.180717 |
| Original priority 15 | main63 | 0.997586 | 0.998376 |

| Workload | Candidate/main | Paired 95% CI |
|---|---:|---:|
| sieve | 1.058626 | [1.000823, 1.094980] |
| binary_trees | 0.995218 | [0.982574, 1.020325] |
| spectral_norm | 0.952152 | [0.890121, 1.023780] |
| mandelbrot | 0.995135 | [0.985373, 1.026362] |
| strlen_bench | 1.018897 | [0.991292, 1.115920] |
| switch_dispatch | 0.977142 | [0.967889, 1.018109] |
| fannkuch | 1.249914 | [1.236342, 1.266423] |
| tls_seg_access | 0.978172 | [0.712457, 0.998601] |
| byte_load_reassembly | 0.896838 | [0.875749, 0.897770] |
| zlib_ng_adler32_combine | 1.003129 | [0.964497, 1.028224] |
| expat_xml_scan | 1.061961 | [0.995421, 1.078867] |
| expat_siphash24 | 1.025245 | [0.879375, 1.122653] |
| sqlite_varint | 1.004282 | [0.982123, 1.075088] |
| linux_find_bit | 0.974121 | [0.957887, 1.021529] |
| linux_find_bit_scaled | 1.013867 | [1.003533, 1.022242] |
| sha256_transform | 0.983186 | [0.922033, 1.009561] |
| zstd_count | 0.963905 | [0.933836, 1.014496] |
| lz4_compress | 1.067006 | [0.904385, 1.154103] |
| lz4_match_extend | 1.015835 | [0.985353, 1.026796] |
| glibc_strstr | 0.995744 | [0.991209, 1.011301] |

Fresh bounded Callgrind: **45/45**, outputs equal, GM Ir **1.001027685**, AM **1.001404478**. Raw counters are preserved, not replaced by rankings.

### Critical follow-through, including inconvenient results

- Fannkuch is **+24.99%** against latest main, CI [1.23634,1.26642]. The earlier same-candidate wrapping ablation remains applicable to identical code. Do not remove semantic guards to hide it.
- Thirty-one-pair follow-ups: sieve **1.048277 [1.037493,1.052551]**; XML scan **1.045092 [1.022639,1.064120]**; scaled LZ4/PASSES4096 **1.026794 [1.001935,1.050531]**. These remain regressions, not acceptable performance claims.
- Same-candidate sieve no-wrap diagnostic: **1.042233 [1.033592,1.048811]**. Assembly shows narrow arithmetic/sign extension in the hot marking loop; Callgrind is +11.87%. A range proof is needed, not an unsafe widening rollback.
- Same-candidate XML affine-fold-disabled diagnostic: **1.035195 [1.020552,1.046243]**. Despite about 1% fewer simulated instructions, the fold/associated layout and allocation changes lose native time. This needs general profitability/layout work; no workload-specific exemption is shipped.
- Linux scaled timing remains **1.015242 [1.000159,1.023480]**, but emitted assembly, all nonzero function symbol bytes/addresses, and simulated counters are identical. Only CRT nop padding differs in text. Do not attribute this VM timing difference to a compiler hot-code change. SipHash flips to **0.973788 [0.953527,1.000232]** on 31 pairs; its emitted assembly/function bytes are also identical.
- Best native point: byte reassembly **0.896838 [0.875749,0.897770]**. There is no demonstrated overall speedup.
- Both refreshed controls stop the boundary suite at the same signed-rotation witness (30 passed, first failure; not a complete control run). Isolated `CCC_LOOP_ROTATE=1 -O2 -fwrapv` replay exits 1 for main and PR831, 0 for candidate. Memcheck reports zero memory errors for all three: this is a wrong-result witness, not a memory-checker diagnosis.
- Preserve main’s independent hosted lanes and stricter oracle tests. Import refreshed PR831 FXRSTOR/conditional-XRSTOR controls and escaped failure annotations. Retain the validated 128-bit reciprocal packer under the upstream intrinsic name; its new upstream runtime gate passes **9 × 9108** comparisons. Its production no-legacy-transition assertion runs with normal VEX promotion; the disabled-pass mode still has runtime coverage.
- The i686 DT_RELENT repair and corrected note-run oracle are retained. The Zstd **8→11 static stack-operand baseline exception** remains explicit; no global tolerance was relaxed.
- **Performance approval remains withheld.** Final full-fast CI status, including intentional slow skips, is in the delivery ledger; the old-base interrupted run is not acceptance evidence.

Raw evidence: `engineering/evidence/pr831-review/latest-main-evidence.tar.xz`; identities and hash in `latest-main-manifest.json`.

---
## Historical 7f9eb225 / initial PR831 measurements

## Identities and scope

- Main: `7f9eb225def23bda9370d3349304b9b785554fea`; PR831 control: `9881506686a270497d635917dde75297ed805a5f`. Independent compiler builds; no shared Cargo target.
- Candidate compiler SHA256: `47cd3acac75c81c414cfa6eda9b3fa04e1ca733e6216a3191b675a15fb0b9ed9`.
- PR831 compiler SHA256: `01503f8b1383b2ec37d2f02730c1c6e9880d07c38230a1785a622c24f28ab5d8`.
- Rust 1.99, MSRV 1.97; compiler builds use fastbuild O1/j2. Six GiB active swap. The ELF comment confirms GCC16.2 + mold2.42.1; the configured Rust linker driver is GCC, not the stale clang comment in the build helper. Mold was built with X86_64/I386 only.
- Local references: GCC16.2.0, Clang/lld23.1.3, Debian GAS/bfd2.47, Valgrind3.27.1. CE: GCC16.2, Clang23.1.0, ICC2021.10; `icxlatest` executes ICX2024.0.0, **not a verified current-2026 release**. Full raw identities and responses are retained.
- Candidate naturally selects GCC16/32 support files; PR831 needed genuine GCC14 multilib because its discovery list ends at 14. This startup/runtime selection difference is disclosed, not described as perfectly identical libraries.

## Correctness and audit results

- 4773 Rust library tests pass, 8 ignored; integration test passes. Full fast-CI status is recorded in the delivery ledger; no slow/full CI was run.
- 265 cross-boundary checks: 0 failures, 0 skips. This includes the new masked-index proof boundary and all four supported architectures.
- 66 native/i686 FP policy/rounding/precision cases pass; all eight recurrence exact-bit/FENV/oracle combinations and the corrected causal assembly contracts pass.
- Nine packing arms ×1260 bitwise cases, each with Memcheck; 24 exact-oracle/ABI/domain Memcheck cases pass.
- RV binding/oracle: candidate 480/0/0 versus PR831 250/230/0. Real RV loader: candidate 13/0/0 versus PR831 3/10/0.
- PR831 additionally segfaults on the wrap witness at all six optimization levels; its dynamic-rounding witness exits 41 at O0 and 21 at the remaining levels.
- New repairs beyond the selective PR831 import: conditional-move/narrowing-conversion read extents; overflow-safe overlap endpoints; installed-version GCC runtime discovery; real float.h evaluation macros; observable-FP backedge PRE fence; signed nonnegative-mask range propagation.
- Synthesis test duplication was repaired, not concealed. The initial GCC16 link failure was a compiler discovery defect, not merely missing infrastructure.
- Existing ill-typed builtin baseline remains 174 ICE / 294 silent accepts (no additions). This is **not** a zero-defect certification.

## Paired native timing

Ratios are candidate/reference; below 1 is faster. Canonical default sizes, O2, x86-64-v3, fwrapv, uniform library/header flags; nine seeded randomized paired rounds, two excluded warmups. Four arms run serially on CPU0 after all compiler builds/CI/provisioning stopped. No outlier removal. These are whole-process **Xeon VM** timings, not the supplied EPYC7763 or a physical Raptor Lake machine. Host frequency/P-E/SMT interference cannot be controlled here; PMU counters report unsupported.

| Set | Reference | GM | AM |
|---|---|---:|---:|
| Expanded 20 | pr831 | 1.011893 | 1.013519 |
| Expanded 20 | gcc | 1.172560 | 1.187410 |
| Expanded 20 | clang | 1.088081 | 1.123201 |
| Original priority 15 | PR831 | 0.996794 | 0.997243 |

| Workload | Priority 15 | /PR831 | Paired 95% bootstrap CI | /GCC16 | /Clang23 |
|---|:---:|---:|---|---:|---:|
| sieve | yes | 1.043948 | [1.016848, 1.090102] | 1.073679 | 1.210409 |
| binary_trees | yes | 0.999464 | [0.970142, 1.031801] | 1.143121 | 1.041843 |
| spectral_norm | yes | 0.982061 | [0.920986, 1.035880] | 1.396078 | 0.716779 |
| mandelbrot | yes | 1.021239 | [0.990873, 1.047999] | 1.200692 | 1.171155 |
| strlen_bench | yes | 0.986502 | [0.910520, 1.055696] | 1.021515 | 1.042527 |
| switch_dispatch | yes | 1.017860 | [0.991644, 1.061222] | 1.220346 | 1.083800 |
| fannkuch | no | 1.230372 | [1.210480, 1.283534] | 1.471938 | 1.234189 |
| tls_seg_access | no | 1.007840 | [0.905429, 1.048974] | 1.046354 | 0.540319 |
| byte_load_reassembly | yes | 0.902618 | [0.880646, 0.931749] | 1.900642 | 2.167943 |
| zlib_ng_adler32_combine | yes | 1.004896 | [0.977521, 1.014143] | 1.024952 | 1.063673 |
| expat_xml_scan | yes | 0.991272 | [0.929684, 1.049822] | 1.207825 | 1.279381 |
| expat_siphash24 | yes | 1.010209 | [0.890724, 1.115955] | 0.968463 | 1.059932 |
| sqlite_varint | yes | 1.001188 | [0.959161, 1.073541] | 1.252297 | 1.051954 |
| linux_find_bit | yes | 1.003556 | [0.803604, 1.187387] | 1.225644 | 1.275465 |
| linux_find_bit_scaled | yes | 0.990656 | [0.983553, 1.059808] | 1.218189 | 1.258110 |
| sha256_transform | yes | 0.998249 | [0.932883, 1.032464] | 1.125747 | 0.948963 |
| zstd_count | no | 1.004486 | [0.996202, 1.015288] | 1.065450 | 1.089859 |
| lz4_compress | no | 1.089790 | [1.041011, 1.215243] | 1.097960 | 1.130338 |
| lz4_match_extend | yes | 1.004929 | [0.963116, 1.021015] | 1.032515 | 1.024875 |
| glibc_strstr | no | 0.979249 | [0.968543, 1.011718] | 1.054783 | 1.072513 |

### Regression diagnosis, not excuses

- **Fannkuch +23.04%** is real in this screen: paired CI [1.21048, 1.28353]. The same candidate with `-fno-wrapv` is faster by a candidate/diagnostic ratio 1.25665 [1.22064, 1.27690]. Assembly shows the wrapping-safe narrow index extension and additional spills. The unsafe-contract ablation is not shipped. Recovering this requires a stronger range/narrow-consumer proof, not removal of wrapping guards. Performance acceptance remains withheld.
- **LZ4 +8.98%** initially measured only 2.93ms, below the runner’s 20ms quality floor. A separate 31-round PASSES=4096 experiment raises candidate median to 68.45ms: ratio **1.025284**, CI [1.009897, 1.048757]. This is a remaining >2% regression, not the original 9% claim.
- Sieve +4.39%, CI [1.01685, 1.09010], remains a priority. Its bounded no-wrap ablation explains the extra narrow arithmetic; nonlinear/loop-variant bounds need proof before widening.
- Mandelbrot, switch dispatch and SipHash point estimates exceed 1%, but their intervals include parity. No workload-specific optimizer exceptions were added.
- Best native result here: byte-load reassembly **0.902618**, CI [0.880646, 0.931749]. The original-priority aggregate near parity is not a statistically established overall win.

## Canonical bounded Callgrind

45/45 output-equal cases in both comparisons. Fixed I1/D1/LL geometry and branch simulation; Ir includes startup/libraries. Bounded inputs (including fannkuch N=7) are **not** the default native sizes (fannkuch N=11). Ir is not cycles, runtime, physical-cache performance, or universal compiler ranking.

| Reference | GM Ir ratio | AM Ir ratio |
|---|---:|---:|
| pr831 | 1.001027702 | 1.001404495 |
| gcc16 | 0.988600778 | 1.165829128 |

TLS is now exactly 1.0 relative to PR831, versus 1.180362 before the signed-mask proof: the previous instruction-count penalty is eliminated without relaxing fwrapv.

| Workload | /PR831 Ir | /GCC16 Ir |
|---|---:|---:|
| arith_loop | 1.000000 | 0.823937 |
| fib | 1.000000 | 0.072949 |
| matmul | 1.001391 | 0.902808 |
| qsort | 1.000000 | 1.033394 |
| sieve | 1.118720 | 1.053997 |
| tce_sum | 1.000000 | 0.952753 |
| nbody | 0.987083 | 1.286266 |
| binary_trees | 1.000000 | 1.079076 |
| spectral_norm | 0.971019 | 2.020587 |
| mandelbrot | 1.000000 | 1.088440 |
| hash_table | 1.000000 | 1.046422 |
| strlen_bench | 1.009298 | 1.138595 |
| switch_dispatch | 1.000000 | 1.081048 |
| struct_copy | 1.000000 | 0.937990 |
| loop_patterns | 1.000010 | 1.250227 |
| fannkuch | 1.026218 | 1.149291 |
| ackermann | 1.000000 | 0.126690 |
| constant_recursion | 1.000000 | 0.126479 |
| bitops | 1.000000 | 0.730699 |
| double_reduction | 1.000000 | 1.125838 |
| ascii_case_fold | 1.000000 | 1.620906 |
| binary_search | 1.002928 | 1.400138 |
| ring_fifo | 1.000000 | 2.477876 |
| aarch64_select_patterns | 1.000000 | 1.213808 |
| histogram | 1.000000 | 1.904683 |
| gzip_crc32 | 1.000000 | 0.991116 |
| libm_round_family | 1.000000 | 0.974934 |
| tls_seg_access | 1.000000 | 0.971999 |
| percpu_ptr_walk | 1.000000 | 0.984984 |
| byte_load_reassembly | 0.875018 | 4.286973 |
| zlib_ng_adler32 | 1.000000 | 0.918115 |
| zlib_ng_adler32_combine | 1.000000 | 1.022451 |
| expat_xml_scan | 0.990014 | 1.150186 |
| expat_siphash24 | 1.000000 | 0.990147 |
| sqlite_varint | 1.021878 | 1.090335 |
| linux_find_bit | 1.000000 | 1.173412 |
| linux_find_bit_scaled | 1.000000 | 1.182398 |
| glibc_memcmp | 1.000000 | 1.044618 |
| chacha20_block | 1.000000 | 0.509358 |
| sha256_transform | 1.006749 | 0.949139 |
| linux_rbtree | 1.000000 | 1.253923 |
| zstd_count | 1.008905 | 1.115910 |
| lz4_compress | 1.007798 | 1.129168 |
| lz4_match_extend | 1.000000 | 1.242343 |
| glibc_strstr | 1.036174 | 1.835904 |

## Compiler Explorer interpretation

All four available vendors pass the common Haswell ISA canary. GCC/Clang/ICX pass the wrap witness under contraction-off flags; ICC rejects that option, so its rejected invocation is not a pass. With vendor-specific `-fp-model strict`, ICC and ICX both pass wrap; ICX passes FENV, ICC exits41. GCC’s optimized FENV witness exits52 while Clang passes. Preserve these disagreements; do not weaken the required semantics or claim four-vendor unanimity. The six-workload static survey preserves actual flags, assembly and errors; static line counts are not performance measurements.

## Reproduction and remaining work

Raw commands, source hashes, paired samples, counters, tool identities, CE replies and key assembly are in `engineering/evidence/pr831-review/fresh-evidence.tar.xz`; its hash and compiler identities are in `fresh-manifest.json`. `native-replay.py --help` reuses the canonical registry/runner and requires explicit compiler paths. See `backlog.md` for concrete remaining proof, query-path and performance tasks. No push or external review certification was performed.

## Final fast-gate follow-through and evidence continuity

The first complete run was **253 pass / 3 fail / 6 slow skips**, not green.
Strict Clippy, fmt and MSRV passed. Its three findings were addressed openly:
1. Repair missing i686 DT_RELENT on PLT-only objects; executed NOTYPE routing
   and dynamic relocation decoding now pass on both x86 ABIs.
2. Repair the note-layout oracle: seven bytes of real rodata in an eight-byte
   gap are not padding. Keep the correct two note segments, test occupied
   intervals/virtual continuity, and require exactly-once note coverage.
3. Accept a **documented static ratchet exception** for Zstd stack operands
   8→11 only; do not raise the global tolerance or call it a speedup. Native
   ratio is 1.004486 [0.996202,1.015288], bounded Ir about +0.89%; the extra
   stores remain a backlog item and performance approval stays withheld.

The delivery compiler SHA256 is
`84d0463930962e007eba27f82ec0423c562a05d58ac6359c233c0d118c2dad94`.
The earlier hash above identifies the **measured** compiler. Ninety complete
x86-64 benchmark executable comparisons (all 45 default/native and all 45
bounded/Callgrind builds) prove byte identity after the i686-only metadata fix;
raw commands/hashes are in `x86-binary-equivalence.json`. Thus the recorded
native and counter measurements apply to identical generated executables.
The final-tree fast verdict is recorded in the delivery ledger, not assumed.

The fast runner also rebuilt hash-pinned binutils 2.47 tools for x86-64 and
AArch64 (reported build token 2.47.20260726). Provenance markers are preserved
with the acceptance artifacts. Optional all-target/all-feature compilation
passed before this final metadata repair and is checked again with the final tree.
