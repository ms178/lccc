# 150 highest-ROI work items

Ranked work for **generated-code performance** vs GCC 16.2 / Clang 22.1 / ICC 2021.10 / ICX.
This file **is** the 150-item catalog (IDs RA / IS / OP / FE / AB / PG / LK / MS). Do not replace it with a stub.

Evidence: **M** measured LCCC vs GCC14 on kernels; **G** Compiler Explorer (`cg162` / `cclang2210` / `cicc2021100` / `cicxlatest`, `-O2 -march=x86-64-v3`); **C** tree fact; **S** screening suite.

**ROI rank:** P0 this week (testable) · P1 this month · P2 after P0/P1 · P3 research.

Count: RA 22 + IS 28 + OP 30 + FE 18 + AB 15 + PG 12 + LK 18 + MS 7 = **150**.

## 1. How to gather data (mandatory)

```text
1. Reproducer C (≤80 lines) in tests/benchmark/programs/ or engineering/evidence/godbolt/
2. scripts/godbolt.py compare FILE --local target/fastbuild/lccc \
     --oracles gcc16.2,clang,icc,icx --function NAME \
     --flags '-O2 -march=x86-64-v3' --artifact-dir results/NAME
3. Count: insns, (%rsp)/(%rbp), GOTPCREL, vfmadd, btq, ymm, push callee-saved
4. If runtime: tests/benchmark/run_benchmarks.py --only KERNEL --reps 15
5. Kill-switch the change; gzip longest_match stack-mem must not rise
6. PMU only on 14700KF after (2)+(4) agree
```

Build: `scripts/ensure_swap.sh` then `scripts/build_lccc_fast.sh` → `target/fastbuild/lccc`.

## 2. Architectural flaws (read before coding)

**A. Two codegens, two allocators.** Text IR → GNU-as + string peephole, and MachInst + a second linear scan (`machinst_regalloc.rs`), off when loop body >32 insts (gzip −3%). Unifying without a cost model repeats that regression.

**B. Location model is a pile of sets.** Homes live in `reg_assignments`, stack slots, `never_materialized`, `immediately_consumed`, `param_pre_stored`, `fused_cmp_dests`, `cmp_replay`, `load_cast_fold`, XMM vecreg, MachInst regs. `ValueLocation` never landed.

**C. RA scans fat intervals.** `liveness.segments` used for call-span only. Scan still `[start,end]`. Diamond CFGs (xmltok/inflate) over-interfere. `split_ranges.rs` is timid IR rewrite, not Wimmer.

**D. Accumulator is still the ISA.** FP struct fields still `movq %xmm,%rax; movq %rax,%xmmN` (`struct_copy` 21.06×).

**E. Sema is a stub.** `src/frontend/sema/analysis.rs`: full type checking TODO. No IR `noalias`/`restrict`.

**F. Alias is loop-SCEV-lite.** `alias.rs` exists; LICM does **not** call it for GEP loads (`licm.rs:750`).

**G. PGO cannot talk to RA.** `CCC_PGO_WEIGHT_MAX` default 1 (gzip +4.7% at 4). Layout must not reorder (expat 131→248 ms). `vectorize_gate` ignores profile (`unroll_pgo.rs:122` `return true`).

**H. Dual coalescers.** RA copy-groups vs `stack_layout/copy_coalescing.rs`.

**I. ExprId is a pointer.** Lowering TODOs; const-eval / typeof suffer.

## 3. Empirical scoreboard (do not “fix RA” for CRC)

| Workload | LCCC vs GCC14 | Root cause | Oracle (CE -O2 v3) |
|----------|---------------|------------|---------------------|
| gzip `longest_match` | 118 vs 0 stack-mem, 248 B frame, GOT | **RA remat + RIP + IV homes** | gcc `window(%r9,%rcx)`, 1× push rbx |
| zlib-ng Adler kernel | **1.49×** | **RA next-use** (`sum2`/`n` on stack in DO8) | icx/clang: 0 stack |
| gzip CRC kernel | **1.47–1.49×**, 2 vs 0 spills | **ISel SIB** `crc_table(,%rcx,4)` | gcc 20-line loop; **no `crc32` insn** |
| Expat name scan | **1.95×** | **classify + inline** | gcc `btq` |
| xmltok.c TU | **12×** stack-mem | **RA segments** | — |
| inflate.c TU | **15×** stack-mem | **RA segments + switch** | — |
| `struct_copy` | **21.06×** (585 vs 28 ms) | **ABI + XMM field copies** | gcc/icx: 2× ymm for 64 B `*dst=*src` |
| `dot` | vectorizer reduction exists | **ICX FMA YMM acc**; gcc16.2 worse | **copy ICX not GCC** |
| nbody / spectral / mandelbrot | **2.9–3.8× / 9.3× S** | **non-reduction vectorize** | ICX nbody 97 ymm / 58 FMA; gcc 0 ymm / 24 FMA |
| sqlite varint | **~2× S**; 189-inst inline **rejected** | **ISel branches**, not inline | — |
| linux_find_bit | **1.85× S** | gcc **`andn`+`cmov`** on C ffs tree, **not tzcnt** | CE corpus |
| bitops | — | gcc/clang `popcntl` | recognize hand-rolled popcount |
| sieve | 1.3× S | gcc **45 scalar**; clang **365 ymm** | **do not copy clang** |
| geomean micros | 0.92× | **fib TCE** — ignore for codecs | — |

## 4. Hard constraints

1. Correctness first: fuzz + `cargo test --lib` + sqlite/expat/gzip if RA/SROA/MachInst.
2. **gzip `longest_match` stack-mem is the RA gate.** Mode 5 eviction and `PGO_WEIGHT_MAX>1` already lost.
3. Do not enable MachInst on large loops without beating the 3% gzip loss.
4. Do not enable `CCC_SROA_COPYOUT` without a dominator proof (hangs `structs_bitfields`, `simd_sse_float`).
5. PhysReg **(11)=`%r10`**, **(10)=`%r11`**.
6. `cmp` must not be in `CCC_X64_NOHOME_CLASSES`.
7. Paths are `src/…`. Binary is `lccc`.
8. Copy **ICX** FMA, not GCC 16.2 per-iter horizontal.
9. Do not vectorize sieve like Clang.

Kill-switches: `CCC_NO_LOAD_CAST_FOLD`, `CCC_NO_X64_IMMED_NOHOME`, `CCC_MI_FORCE_LOOPS`, `CCC_MI_MAX_LOOP_INSTS`, `CCC_SROA_COPYOUT`, `CCC_EVICT_MODE`, `CCC_NO_COALESCE`, `CCC_DEBUG_RA`, `CCC_DUMP_IR`, `CCC_NO_PHI_COALESCE`, `CCC_NO_LEAF_PARAM_GPR`, `CCC_NO_FOLDED_INDEX_LIVENESS`, `CCC_NO_LOAD_HAZARD_REFINE`, `CCC_NO_EAX_ALLOC`, `CCC_NO_LOOP_PIN`, `CCC_NO_VECREG`, `CCC_PGO_WEIGHT_MAX`, `CCC_TRACE_ALLOC`, `CCC_X64_NOHOME_CLASSES`, `CCC_NO_MACHINST`.

---

## 5. The 150 items

Each row: `ID | P | item | files | evidence | accept | do-not`.

### 5.1 Register allocation — RA-01 … RA-22

| # | ID | P | Item | Files | Evidence | Accept | Do not |
|---|----|---|------|-------|----------|--------|--------|
| 1 | RA-01 | P0 | Remat file-scope `window`/`prev`/`strstart` as `sym(%rip)` / `sym(%rip,idx,1)` not GOT+stack | `globals.rs`, `live_range.rs`, `generation.rs` | **M** 118 stk; **G** gcc `window(%r9,%rcx)` | match-mini CE: 0 GOT, ≤1 push | PIC/TLS still GOT |
| 2 | RA-02 | P0 | Keep match IVs (`scan`,`best`,`chain`) in GPR | `live_range.rs` next-use | **M** 248 B frame | stack-mem &lt;20 on gzip `longest_match` | eviction mode 5 global |
| 3 | RA-03 | P0 | Adler DO8: evict dead bytes not `sum2`/`n` | `live_range.rs` `future_uses` | **M** 72(%rsp) in loop; **G** icx 0 stk | Adler kernel ≤1.15× GCC | unify incoming/victim cost blindly |
| 4 | RA-04 | P0 | `CCC_RA_EXPLAIN=fn` dump | `regalloc.rs` | **C** | one line per spill | dump that changes allocation |
| 5 | RA-05 | P1 | Segment-aware interference | `liveness.rs` segments → scan | **M** xmltok 12× inflate 15× | those ratios &lt;4× | one-home codegen without reloads |
| 6 | RA-06 | P1 | Second-chance reload; make `enable_splitting` real | `live_range.rs`, codegen reload | **C** stub `false` | gzip unregressed | IR volatile alloca dance only |
| 7 | RA-07 | P1 | Call-site caller-saved save vs always r12 | `regalloc.rs` `caller_save_spans` | **C** Phase 2b slots | use spans more; prologue stk down | promote r11 to callee-saved |
| 8 | RA-08 | P1 | Affinity coalescing overlapping copies | `regalloc.rs` `build_coalesce_groups` | **C** disjoint-only | fewer movs, no extra spills | merge overlapping intervals |
| 9 | RA-09 | P2 | Residual Briggs on spilled | new; **not** `stack_layout/graph_coloring.rs` | **C** that file colors **slots** | fewer reloads | rename that file as a GPR allocator |
| 10 | RA-10 | P2 | GPR spill to XMM | new class | ICX/ICC | gzip gate | clobber SSE ABI |
| 11 | RA-11 | P1 | One `RangeMetadata` per function | `live_range.rs` | **C** walk × waves | compile time | change eviction semantics |
| 12 | RA-12 | P1 | Set or delete `exchange_eviction` on 2c | `regalloc.rs` vs `live_range.rs` | **C** drift | gzip A/B | default mode 5 |
| 13 | RA-13 | P1 | `verify_no_overlap` die-at-birth | `regalloc.rs` | **C** | debug assert on overlap | ship as silent skip |
| 14 | RA-14 | P2 | Stronger `split_ranges` (phis, multi-exit) | `split_ranges.rs` | **C** fail-closed | more splits, gzip gate | mem2reg on split slots |
| 15 | RA-15 | P1 | Param group priority remaining holes | `bump_coalesce_group_priority` | **C** adler/memcmp history | fewer param spills | break SysV arg regs |
| 16 | RA-16 | P2 | Recalibrate PGO RA weights **after** RA-01 | `pgo_weight_max` | **C** +4.7% gzip at 4 | gzip + kernels | raise max first |
| 17 | RA-17 | P1 | Folded GEP **base** liveness x86 remaining | `prologue.rs` ~780 `collect_gep_fold_base_links` | **C** gz_reset NULL-store | no NULL-store miscompile | extend folded **indices** on x86 |
| 18 | RA-18 | P2 | MachInst RA vs IR RA unification | `machinst_regalloc.rs` | **C** two scans | one allocator | force MI on gzip |
| 19 | RA-19 | P1 | PhysReg map table in one file | `emit.rs` vs comments | **C** r10/r11 swap | single source of truth | silent remumber |
| 20 | RA-20 | P2 | i686 eax/ecx model → clobber oracle from emit | `regalloc.rs` 2d/2e | **C** | i686 tests | copy x64 policy |
| 21 | RA-21 | P2 | Vecreg whitelist vs new SIMD ops | `collect_vecreg_candidates` | **C** fail-closed | test per intrinsic | open whitelist |
| 22 | RA-22 | P3 | PBQP irregular (AArch64) | — | after RA-01–06 | — | before x64 codecs win |

### 5.2 x86 ISel / peephole / MachInst — IS-01 … IS-28

| # | ID | P | Item | Files | Evidence | Accept | Do not |
|---|----|---|------|-------|----------|--------|--------|
| 23 | IS-01 | P0 | CRC `xor table(,%rcx,4), %eax` | `memory.rs` / ALU | **G** gcc; **M** 1.47× 2 vs 0 stk | SIB in crc kernel | emit `crc32` insn (oracles do not at -O2) |
| 24 | IS-02 | P0 | Keep `double` fields in XMM; ban `movq %xmm,%rax` roundtrip | `float_ops.rs`, `memory.rs` | **S** 21× struct_copy | mov count ≤40 | keep rax shuttle |
| 25 | IS-03 | P0 | 64 B assignment → 2× `vmovdqu ymm` | memcpy path **exists**; assignment may not | **G** gcc 7 insns | `copy_s` matches gcc/icx | claim memcpy missing |
| 26 | IS-04 | P0 | Vectorizer emit `vfmadd231pd` into YMM acc **once** horiz | `vectorize.rs` + `intrinsics.rs` | **G** **icx** not gcc16; spectral ICX FMA 10 | `dot`/spectral CE vs icx | copy gcc16 per-iter hadd |
| 27 | IS-05 | P1 | Mem-op `vmulpd (%rdi),…` | ISel fold | **G** | mem operand in vec loop | fold aliased |
| 28 | IS-06 | P1 | `btq` / bitset classify | `simplify.rs` + emit | **G** gcc name_scan; **M** 1.95× | `btq` in name_scan | mega-inline varint |
| 29 | IS-07 | P1 | `cmov` limit idiom | `if_convert.rs` | **G** gcc match `cmovc` | cmov in match | if-convert huge blocks |
| 30 | IS-08 | P1 | Load-cast fold remaining | `prologue.rs` load_cast | **C** 33% gzip-9 was movzbl | leftover movzbl down | disable `CCC_NO_LOAD_CAST_FOLD` globally without measure |
| 31 | IS-09 | P1 | Cmp-replay only from **slots** | keep | **C** sqlite `yy_shift` | no sqlite SEGV | replay from physreg |
| 32 | IS-10 | P1 | Flag fusion Copy/Cast chain | exists; extend carefully | **C** expat accounting SEGV if multi-use | more fused cmp | multi-use flags |
| 33 | IS-11 | P1 | C `__ffs` if-tree → gcc **`andn`+`cmov` chain** (not tzcnt) | `bit_idioms.rs`, `alu.rs` | **G** gcc cmov×11, **not** tzcnt; 1.85× | find_bit CE vs gcc | blindly emit tzcnt |
| 34 | IS-12 | P0 | `a & ~b` → `andn` / `andnq` | ALU BMI2 | **G** find_bit gcc `andn` icx `andnq` | CE find_bit | require tzcnt |
| 35 | IS-13 | P1 | ALU+mem for spilled | peephole | ICX lookalike | fewer reloads | change RA homes |
| 36 | IS-14 | P1 | Dead `movq %r,%r` | peephole | **C** | 0 identity mov | delete live copies |
| 37 | IS-15 | P1 | Redundant `movslq` on indices | narrow vs backend | **C** DOOM −390; rest is GEP | fewer movslq | break GEP signedness |
| 38 | IS-16 | P2 | Match inner 258 B with `pcmpeqb` | vectorize | spike | gzip gate | ship without CRC/adler oracles |
| 39 | IS-17 | P1 | MachInst profit model ≠ 32-inst cutoff | `prologue.rs:812` | **C** gzip −3% | beat gzip then raise | `CCC_MI_FORCE_LOOPS` |
| 40 | IS-18 | P2 | MachInst fused cmp/select | declined | **C** | — | force on gzip |
| 41 | IS-19 | P2 | Text peephole vs MachInst conflict | `CCC_NO_MACHINST` | **C** | documented interaction | dual-apply same fold |
| 42 | IS-20 | P1 | `vzeroupper` only if YMM dirtied | memcpy | **G** gcc emits | AVX transition safe | always vzeroupper |
| 43 | IS-21 | P2 | Scheduler already lost gzip 3% | emit / prologue | **C** `-mtune` diagnostic only | PMU after ISel | turn on as default |
| 44 | IS-22 | P1 | Indexed GEP SIB completeness | generation + memory | **S** loop_patterns 1.85× | SIB in loops | GOT for file-scope |
| 45 | IS-23 | P2 | `repe cmps`/`pcmpeqb` memcmp | | noisy &lt;20 ms | scale work first | optimize 20 ms kernel |
| 46 | IS-24 | P1 | Immediate stores; audit remaining xor+movb | memory | **C** | fewer xor+movb | |
| 47 | IS-25 | P2 | 3-channel mul ILP (`exclude_every_third_mul_temp`) document or kill | `regalloc.rs` | **C** magic | documented or gone | silent heuristic |
| 48 | IS-26 | P1 | GlobalAddr fold vs GOT (`needs_got_for_addr`) | never_materialized | **C** | RIP for file-scope | PIC/TLS |
| 49 | IS-27 | P2 | i128 through rdx:eax without excluding r8–rsi whole fn | prologue i128 filter | **C** | more GPR | ABI break |
| 50 | IS-28 | P0 | Hand-rolled popcount → `UnaryOp::Popcount` (`alu.rs` already emits `popcntl`) | `bit_idioms.rs`, `alu.rs` | **G** bitops gcc/clang `popcntl` | bitops uses insn | add a second popcnt emitter |

### 5.3 Optimizer / alias / SROA / vectorize — OP-01 … OP-30

| # | ID | P | Item | Files | Evidence | Accept | Do not |
|---|----|---|------|-------|----------|--------|--------|
| 51 | OP-01 | P0 | LICM GEP loads via `alias.rs` `forms_disjoint` | `licm.rs:750` | **C** TODO vs existing engine | more hoists, no alias miscompile | invent second alias |
| 52 | OP-02 | P1 | Dominator-aware SROA copy-out | `aggregate_sroa.rs` `CCC_SROA_COPYOUT` | **C** hangs 2 tests | those tests + struct_copy | enable flag without proof |
| 53 | OP-03 | P1 | SROA cross-block memcpy (same-block only now) | same | **C** | more scalarized fields | ignore dominance |
| 54 | OP-04 | P0 | `vectorize_gate` real cost (trip, size, PGO) | `unroll_pgo.rs:122` | **C** always true; **G** clang sieve 365 ymm vs gcc 45 | no 4-iter vec; **no clang-sieve** | copy clang sieve |
| 55 | OP-05 | P0 | Non-reduction FP loop vectorize (nbody YMM) | `vectorize.rs` (~6905) | **S** nbody 3.8× spectral 9.3×; **G** ICX nbody 97 ymm / 58 FMA vs gcc 0 ymm / 24 FMA | ymm+FMA vs ICX | copy gcc16 horiz-per-iter |
| 56 | OP-06 | P0 | Horiz reduce **once** (ICX) not per-iter (gcc16) | vectorize | **G** | one hadd/pack at exit | gcc16 pattern |
| 57 | OP-07 | P1 | Runtime alias versioning | none | **C** | versioned loops, checksums | version everything |
| 58 | OP-08 | P1 | Nested IVSR with dependence | `iv_strength_reduce.rs` skip overlap | **C** nested-matmul wrong if forced | loop_patterns + matmul | force overlapping |
| 59 | OP-09 | P2 | Enable `univsr` after SSA repair | gated | **C** ideas | | enable gated pass raw |
| 60 | OP-10 | P1 | Unroll vs RA pressure | `loop_unroll.rs` | **C** | gzip unregressed | unroll longest_match blindly |
| 61 | OP-11 | P2 | Loop interchange matmul jk | vectorize / loop | **S** | matmul vs ICX | interchange dependent |
| 62 | OP-12 | P1 | Load CSE / GVN less conservative | `gvn.rs` | **C** | fewer redundant loads | CSE aliased |
| 63 | OP-13 | P1 | DCE non-escaping dead stores | `dce.rs` all Store live | **C** | dead stores gone | DCE volatile |
| 64 | OP-14 | P1 | If-convert &gt;8 inst / more diamonds | `if_convert.rs` | **C** | more cmov | convert huge blocks |
| 65 | OP-15 | P2 | `range_check.rs` → `btq` | range_check | wire to IS-06 | btq classify | |
| 66 | OP-16 | P1 | `bit_idioms.rs` coverage (popcnt, andn, ffs) | bit_idioms | **G** | IS-11/12/28 fire | |
| 67 | OP-17 | P2 | `reassoc_accum.rs` vs FMA | reassoc | | FMA-friendly assoc | break fp assoc at -O2 default |
| 68 | OP-18 | P2 | `fp_const_hoist` / `int_const_hoist` audit | | **C** | | |
| 69 | OP-19 | P1 | `load_forward` / `store_load_forward` vs GVN overlap | | compile-time | no double work | |
| 70 | OP-20 | P1 | `loop_memory_promote` more consumers | alias.rs | **C** | more promotions | ignore alias |
| 71 | OP-21 | P2 | `quadratic_sr` | | | | |
| 72 | OP-22 | P1 | `redundant_loads` | | | | |
| 73 | OP-23 | P2 | `block_layout` vs PGO conservative | **M** expat 131→248 | don’t reorder | layout reorder hot loop |
| 74 | OP-24 | P2 | `outline_switch` threshold is **40**; measure size vs gcc | `outline_switch.rs:37` | **C** not 999999 | size vs gcc | set 999999 |
| 75 | OP-25 | P1 | Inline `name_cont` | `inline.rs` | **G** gcc inlines | expat scan | 189-inst varint spike |
| 76 | OP-26 | P1 | Pressure-aware inline | inline.rs | sqlite | no +1119 B 1.00× | |
| 77 | OP-27 | P2 | IPCP function cloning | `ipcp.rs` | | | clone explosion |
| 78 | OP-28 | P3 | SCEV full (subsume IVSR+alias+licm) | | high risk | | rewrite all three at once |
| 79 | OP-29 | P1 | Pass order experiment with gzip/adler/expat oracles | `src/passes/mod.rs` | **C** dirty skip | measured order | shuffle without oracles |
| 80 | OP-30 | P2 | Keep `-O0` vs `-O2` distinct | `src/passes/README.md` | **C** tiers real | tests at both | assume all -O identical |

### 5.4 Frontend / IR — FE-01 … FE-18

| # | ID | P | Item | Files | Evidence | Accept | Do not |
|---|----|---|------|-------|----------|--------|--------|
| 81 | FE-01 | P1 | Sema type diagnostics (not lowerer) | `sema/analysis.rs` | **C** TODO; ~1 vs ~382 | diagnostics in sema | silence lowerer checks first |
| 82 | FE-02 | P1 | TBAA / `restrict` → IR | IR attrs | **C** noalias grep empty | attribute on IR | pretend alias is TBAA |
| 83 | FE-03 | P2 | ExprId not pointer | lowering | **C** TODOs | stable ids | |
| 84 | FE-04 | P2 | Vector subscript | frontend | **C** | | |
| 85 | FE-05 | P2 | typeof diagnostic | | **C** | | |
| 86 | FE-06 | P2 | `__builtin_nan` payload | | **C** | | |
| 87 | FE-07 | P2 | Parser panic-mode | ideas | | | |
| 88 | FE-08 | P2 | `_Complex long double` | | **C** | | |
| 89 | FE-09 | P2 | Unsigned subint IR | `ideas/fix_ir_unsigned_subint*` | **C** | | |
| 90 | FE-10 | P1 | Alignment/range on IR for RA | IR | **C** | better homes | |
| 91 | FE-11 | P2 | `fastcall` fptrs | | **C** | | |
| 92 | FE-12 | P2 | Nested sizeof | | **C** | | |
| 93 | FE-13 | P3 | Use-def shared context | ideas high_use_def | **S** backlog | O(1) use | rewrite every pass at once |
| 94 | FE-14 | P2 | Builtin table unify sema/type_checker | | **C** | | |
| 95 | FE-15 | P2 | C11 UCN lexer | | **C** | | |
| 96 | FE-16 | P1 | Volatile through peephole | prologue `LCCC_VOLATILE_SLOT` | **C** | volatile preserved | fold volatile |
| 97 | FE-17 | P2 | va_arg_pack stub | | **C** | | |
| 98 | FE-18 | P2 | Dependency file `-M` only source | pipeline TODO | **C** | | |

### 5.5 ABI / aggregates / stack — AB-01 … AB-15

| # | ID | P | Item | Files | Evidence | Accept | Do not |
|---|----|---|------|-------|----------|--------|--------|
| 99 | AB-01 | P0 | SysV SSE-class return/args in XMM not rax shuttle | ABI + float_ops | **S** struct_copy 21× | XMM class | rax roundtrip |
| 100 | AB-02 | P0 | sret + one wide copy | ABI | same | one ymm/xmm copy | field loop |
| 101 | AB-03 | P1 | 4-byte slots without zext bug | stack_layout README | **C** | | |
| 102 | AB-04 | P1 | Overalign param copy already special-cased; audit | prologue Alignas(32) | **C** | | |
| 103 | AB-05 | P1 | Parallel copy param prestores remaining | prologue gpr_prestores | **C** | fewer cycles | |
| 104 | AB-06 | P2 | RISC-V va_arg long double struct | current_tasks | **C** | | |
| 105 | AB-07 | P2 | i686 double param high word | current_tasks | **C** | | |
| 106 | AB-08 | P2 | ARM KVM VA layout | ideas | **C** | | |
| 107 | AB-09 | P1 | Callee-save + frame compact offsets | peephole | **C** | smaller frames | |
| 108 | AB-10 | P2 | Postgres frame size | ideas | **C** | | |
| 109 | AB-11 | P1 | Alloca escape analysis more GEP | stack_layout | **C** | | |
| 110 | AB-12 | P2 | Two coalescers unify | RA vs stack | **C** | one policy | |
| 111 | AB-13 | P2 | i686 boot 32K | external_tools TODO | **C** | | |
| 112 | AB-14 | P2 | F128 80-bit x86 | ideas | **C** | | |
| 113 | AB-15 | P1 | Dead XMM save in integer varargs remaining | vararg_fp_save | **C** | | |

### 5.6 PGO / IPO / layout — PG-01 … PG-12

| # | ID | P | Item | Files | Evidence | Accept | Do not |
|---|----|---|------|-------|----------|--------|--------|
| 114 | PG-01 | P2 | Sample PGO | ideas high_sample | **C** | | |
| 115 | PG-02 | P2 | Loop-versioned devirt | ideas | **C** | | |
| 116 | PG-03 | P1 | vectorize_gate use profile | OP-04 | **C** | | ignore profile |
| 117 | PG-04 | P1 | RA use block counts after RA-01 | pgo_weight | **C** | gzip gate | `PGO_WEIGHT_MAX>1` first |
| 118 | PG-05 | P2 | Value specialization | ideas | **C** | | |
| 119 | PG-06 | P1 | Keep conservative layout | **M** expat | no 131→248 | reorder hot loop |
| 120 | PG-07 | P2 | LTO | none | | | |
| 121 | PG-08 | P1 | Force-inline budget vs frame | **C** 8B/SSA | | sqlite spike |
| 122 | PG-09 | P2 | Switch profile density vs bt | switch_dispatch 1.40× | **S** | | |
| 123 | PG-10 | P2 | Function placement | | | | |
| 124 | PG-11 | P1 | Flat-profile gate keep | integrated PGO | **M** adler −17% pre-fix | keep `has_spread` | drop gate |
| 125 | PG-12 | P1 | Don’t promote 95% single-target | `LCCC_PGO_PROMOTE_STABLE=95` | **M** +28% if promoted | keep | promote stable |

### 5.7 Linker / assembler / multi-arch — LK-01 … LK-18

| # | ID | P | Item | Files | Evidence | Accept | Do not |
|---|----|---|------|-------|----------|--------|--------|
| 126 | LK-01 | P2 | mmap I/O | ideas | | | |
| 127 | LK-02 | P2 | `--icf` | linker | | | |
| 128 | LK-03 | P2 | RISC-V PIC auipc | **C** | | | |
| 129 | LK-04 | P2 | ARM `caspal`, `.org`, PREL64, movw | current_tasks | **C** | | |
| 130 | LK-05 | P2 | RISC-V dash FAIL | current_tasks | **C** | | |
| 131 | LK-06 | P2 | RISC-V kernel boot hang | ideas | **C** | | |
| 132 | LK-07 | P2 | `-mno-sse` full | mod.rs TODO | **C** | | |
| 133 | LK-08 | P2 | Encoder completeness | `encoder/mod.rs` TODO | **C** | | |
| 134 | LK-09 | P2 | i686 archive merge | TODO | **C** | | |
| 135 | LK-10 | P2 | R_X86_64_32 PIC | TODO | **C** | | |
| 136 | LK-11 | P2 | RISC-V `@cc` | TODO | **C** | | |
| 137 | LK-12 | P2 | RVV masked | TODO | **C** | | |
| 138 | LK-13 | P2 | Multi-alt asm ARM/RISCV | TODO | **C** | | |
| 139 | LK-14 | P1 | `encdiff.py` / `insndiff.py` in RA ISel loop | scripts | **C** | | |
| 140 | LK-15 | P2 | ELF32 script setup.elf | ideas | **C** | | |
| 141 | LK-16 | P2 | `--emit-relocs` | | | | |
| 142 | LK-17 | P2 | pcre2 792-nest frame | current_tasks | **C** | | |
| 143 | LK-18 | P3 | QEMU build | ideas | **C** | | |

### 5.8 Measurement / process / compile-time — MS-01 … MS-07

| # | ID | P | Item | Files | Evidence | Accept | Do not |
|---|----|---|------|-------|----------|--------|--------|
| 144 | MS-01 | P0 | Godbolt compare in CI for 6 kernels | `scripts/godbolt.py` | procedure §1 | CI artifact | skip oracles |
| 145 | MS-02 | P0 | Spill-count metric `CCC_TRACE_ALLOCSTATS` aggregate | RA | **C** | number in logs | |
| 146 | MS-03 | P1 | CI correctness+regression+fuzz smoke | ideas README | **C** | | |
| 147 | MS-04 | P1 | OnceLock env knobs break parameterized tests | live_range.rs | **C** | tests independent | |
| 148 | MS-05 | P2 | Compile-time: pass rescans | use-def FE-13 | **C** | | |
| 149 | MS-06 | P1 | Document PhysReg map once | RA-19 | **C** | | |
| 150 | MS-07 | P0 | Re-run Adler/CRC/Expat/gzip `-S` on **current `main`** | workloads | **must** | numbers on this SHA | cite older 118 without re-measure |

---

## 6. First 10 tickets (same as SEQUENCE.md)

1. **MS-07** re-measure `longest_match` stack-mem on current SHA.
2. **RA-04** explain dump.
3. **RA-01 + IS-26** RIP-relative `window`.
4. **RA-02 + RA-03** next-use (Adler + match).
5. **IS-01** CRC SIB.
6. **IS-12 + IS-28** `andn` + popcnt IR.
7. **IS-02 + AB-01** XMM fields / SysV SSE class.
8. **IS-03** 64 B ymm assignment.
9. **IS-04 + OP-05** ICX FMA + non-reduction vec (nbody ymm).
10. **IS-06 + OP-01** `btq` + LICM←`alias.rs`.

Stop and re-oracle after each. Gzip match stack-mem is the veto.

## 7. File map

```
src/backend/regalloc.rs               3612  RA policy
src/backend/live_range.rs             1261  scan
src/backend/liveness.rs               2163  segments
src/backend/split_ranges.rs           1377  IR split
src/backend/generation.rs             3622  ISel driver
src/backend/x86/codegen/prologue.rs   1813  RA call, MachInst gate, folds
src/backend/x86/codegen/memory.rs     memcpy ymm
src/backend/x86/codegen/float_ops.rs  FMA scalar
src/backend/x86/codegen/alu.rs        lzcnt/tzcnt/popcnt emitters
src/backend/x86/codegen/machinst*.rs  dual codegen
src/passes/vectorize.rs               6905
src/passes/inline.rs                  3719
src/passes/aggregate_sroa.rs          1009
src/passes/alias.rs                   loop SCEV-lite
src/passes/licm.rs:750                TODO GEP alias
src/passes/outline_switch.rs:37       MIN_CASES = 40
src/pgo/unroll_pgo.rs:122             vectorize_gate = true
scripts/godbolt.py                    CE oracle
engineering/evidence/workloads/
```

## 8. What “beat ICX” means

- **Integer codecs:** remat+RA+SIB (match, Adler, CRC) then ICX XMM-spill if still behind.
- **FP:** copy **ICX** `vfmadd231pd` YMM accumulator, **not** GCC 16.2 per-iter horizontal.
- **Aggregates:** SysV register class + ymm copies (21× is the largest **S** gap).
- **Parsers:** `btq`/range, segment RA, do **not** mega-inline (varint experiment failed).
- **Bit kernels:** gcc `andn`+`cmov` on find_bit; `popcntl` on bitops; **do not** copy clang sieve ymm.
