# P0/P1 high-ROI execution portfolio

This is a ranked execution queue, not a claim that any item is implemented.
Every entry names its owning stage, concrete hypothesis, proof gate, and
success metric.  The ordering follows the repository's current backlog and
rules: correctness and observability precede allocator policy changes.

| # | Item | Owner | Concrete next fix / experiment | Required proof and success metric |
|---:|---|---|---|---|
| 1 | CC-O0CALL-1 | x86 memory emission | Keep accumulator-address loads off the non-SSA `-O0` path. | Csmith reproducer + reduced regression; `cargo test --lib`; GCC output/exit match. |
| 2 | RA-PRESSURE-3 | phi copies | Land a generic cycle resolver for the SHA rotation web. | Generated 2/3/4-cycle mutation tests; SHA copies/spills fall without a gzip stack-reference increase. |
| 3 | RA-GLA-04 | GLA/RA feedback | Record post-RA slot traffic per planned location piece before reopening spill gaps. | Machine-readable feedback and paired sha/sqlite/expat A/B; no regression. |
| 4 | RA-01 | linear scan | Add opt-in allocation-order policies with identical diagnostic counters. | Compare spills, copies, evictions, occupancy, and runtime on the canonical corpus. |
| 5 | RA-02 | linear scan | Trace and simplify position-relative spill cost from use weights to eviction. | Explain report for every eviction; retain only a model that wins paired workloads. |
| 6 | RA-04 | phi webs | Emit physical-home and copy-cost diagnostics for each phi web. | Reduced copies with no extra interference failures or SHA spill increase. |
| 7 | SPILL-01 | stack traffic | Classify every hot stack reference by cause. | JSON classification census; classification must cover all principal workload references. |
| 8 | SPILL-02 | reload placement | Compare reload-at-use, block-entry, earliest-profitable, and remat. | Output-checked runtime plus instruction/stack-reference measurements. |
| 9 | ADDR-01 | address selection | Attach target-neutral BASE/INDEX/SCALE/etc. roles to operands. | x86 legality tests for RSP index, RBP frame, scale, and displacement boundaries. |
| 10 | ADDR-02 | RA policy | Charge/register-credit address-enabling homes through a target policy interface. | A/B address-generation, load/store, and runtime metrics; no benchmark-specific weights. |
| 11 | ADDR-03 | shared representation | Share staged-address opportunity data from IR through lowering. | Mutation tests for aliasing, flags, width, overflow, and implicit operands. |
| 12 | PF-LZ4-1 | loop idioms | Prove both byte ranges before any match-length widening. | Page-edge sanitizer, forward-overlap oracle, four differential oracles, paired target run. |
| 13 | PF-MB-1 | FP loop | Classify scalar Mandelbrot recurrence and find a legal ILP/scheduling change. | Pixel-output oracle and output-checked paired runtime; no speculative cross-pixel vectorization. |
| 14 | PF-FB-1 | CFG/isel | Model find-bit branch/index shape rather than add a test-removal peephole. | Defined zero/nonzero boundary tests and paired runtime better than noise. |
| 15 | PF-SCHEDULER-1 | vectorizer | Implement sliding-window message-schedule recognition. | Verify unaligned/overlap legality; SHA schedule vectorized at -O2 and kernel <=1.15x. |
| 16 | PF-CHACHA-1 | ARX | Separate state permutation, recurrence schedule, and vector schedule diagnostics. | Static and output-checked runtime comparisons against ICX; no special-case resolver. |
| 17 | PF-TLS-1 | x86 lowering | Lower legal local-exec accesses to `%fs:symbol@TPOFF`. | Static/dynamic TLS model matrix and relocation oracle; <=1.05x target goal. |
| 18 | PF-CLS-1 | CFG/vectorizer | Split last-member critical edges before if-conversion/vectorization. | Expat output oracle and branch/schedule measurement, not instruction count alone. |
| 19 | RA-PRESSURE-4 | aggregate RA | Classify aggregate-copy spill webs before changing split policy. | Struct-copy checksum and spill census; Raptor-Lake <=1.05x goal. |
| 20 | MI-CLOBBER-1 | MachInst | Model explicit/implicit defs and clobbers before admitting ordinary Call. | ABI and fixed-register differential suite; never weaken broad call rejection first. |
| 21 | MI-XMM-1 | MachInst/RA | Extend machine register-class modelling to XMM spill/load forms. | Float/vector corpus and AVX/SSE transition gates. |
| 22 | MI-PARAM-1 | MachInst | Lower remaining emissive ParamRef forms with incoming-register identity preserved. | Parameter alias/caller-save matrix and output checks. |
| 23 | SIMD-02 | vector RA | Measure XMM copy webs and add affinity only for proven surviving copies. | Vector-copy census plus mixed SSE/VEX safety gate. |
| 24 | ABI-02 | call-aware RA | Cost caller-save spill versus callee-save save/restore by weighted call frequency. | ABI differential tests and hot/cold call workload A/B. |
| 25 | PGO-01 | profile flow | Audit profile ingestion through RA, scheduling, layout, unroll, and inlining. | Numerical trace and gzip regression guard; do not increase weight range blindly. |
| 26 | LOOP-01 | loop analysis | Explicitly classify recurrence, reduction, invariant, IV, address recurrence, and partial phi. | Classification snapshot tests; feed one consumer at a time with A/B evidence. |
| 27 | SCHED-01 | scheduler | Define target-neutral latency/throughput/dependency/pressure interface. | Model unit tests; no global scheduling policy change yet. |
| 28 | SCHED-02 | scheduler | Prototype critical-path-first scheduling only for selected hot loops. | SHA/ChaCha/CRC/reduction/zlib/zstd correctness and paired timing. |

## Execution discipline

Each candidate must receive its own `engineering/evidence/<TASK-ID>/` record
before policy changes.  VM results are screening-only; target hardware claims
remain **UNVERIFIED ON TARGET** until measured on the i7-14700KF.
