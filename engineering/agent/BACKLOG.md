# 150 highest-ROI work items

Ranked work for **generated-code performance** vs GCC 16.2 / Clang 22.1 / ICC 2021.10 / ICX.
This file **is** the 150-item catalog (IDs P0 / RA / IS / OP / FE / AB / PG / LK / MS). Do not replace it with a stub.

Evidence: **M** measured LCCC vs GCC14 on kernels; **G** Compiler Explorer (`cg162` / `cclang2210` / `cicc2021100` / `cicxlatest`, `-O2 -march=x86-64-v3`); **C** tree fact; **S** screening suite.

**ROI rank:** P0 this week (testable) · P1 this month · P2 after P0/P1 · P3 research.

Count: P0 5 + RA 25 + IS 28 + OP 33 + FE 22 + AB 15 + PG 12 + LK 18 + MS 9 = **167** (the extra 17 are refinements/additions over the original 150; drop the `[NEW]`/`[REFINED]` tags when stable).

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

**A. Two codegens, one dead allocator.** Text IR → GNU-as + string peephole, and MachInst ISel/emit + a **dead** second linear scan (`machinst_regalloc.rs`, 635 LOC, zero callers). The MachInst ISel/emit path is gated off when loop body >32 insts (gzip −3%). Deleting the dead RA (P0-01) is zero-risk; unifying the ISel paths without a cost model repeats the gzip regression.

**B. Location model is a pile of sets.** Homes live in `reg_assignments`, stack slots, `never_materialized`, `immediately_consumed` (hard blocker — RA-23), `param_pre_stored`, `fused_cmp_dests`, `cmp_replay`, `load_cast_fold`, XMM vecreg, MachInst regs. `ValueLocation` never landed. `SlotAddr::Indirect(StackSlot(0))` is a dummy for register-homed values — convention, not enforced (RA-24).

**C. RA scans fat intervals.** `liveness.segments` (hole-aware) is consumed by `regalloc.rs` for call-spanning detection and interval extension (lines 571, 785), but the linear scan itself (`live_range.rs`) still runs on fat `intervals` (single `[start,end]`). Diamond CFGs (xmltok/inflate) over-interfere. `split_ranges.rs` is now a correct IR rewrite (rewrites uses, phi-safe), but is not Wimmer-style in-place splitting.

**D. Accumulator is still the ISA.** FP struct fields still `movq %xmm,%rax; movq %rax,%xmmN` (`struct_copy` 21.06×). `immediately_consumed` hard-codes the accumulator load order (RA-23).

**E. Sema is a stub.** `src/frontend/sema/analysis.rs:15`: "This pass does NOT reject programs with type errors (yet); it collects information for the lowerer. Full type checking is TODO." 13 diagnostics in sema (up from 1) — still a stub. No IR `noalias`/`restrict`.

**F. Alias is loop-SCEV-lite, underutilized.** `alias.rs` (128 LOC) has `LoopFrames`, `resolve_in_frame`, `forms_disjoint`. Consumed by `redundant_loads` only. LICM does **not** call it for GEP loads (`licm.rs:750` TODO). `loop_memory_promote.rs` has its own duplicated `affine_disjoint` engine (RA-25 unifies).

**G. PGO cannot talk to RA.** `CCC_PGO_WEIGHT_MAX` default 1 (gzip +4.7% at 4). Layout must not reorder (expat 131→248 ms). `vectorize_gate` ignores profile (`unroll_pgo.rs:122` `return true`).

**H. Dual coalescers.** RA copy-groups vs `stack_layout/copy_coalescing.rs`.

**I. ExprId is a pointer.** Lowering TODOs; const-eval / typeof suffer.

**J. Lifetime demotion is the only spill model.** `live_range.rs:26-27`: "There is no reload-at-next-use in this module." 3-5× spill traffic vs LLVM. `segments` infrastructure is built but the scan doesn't use it for splitting (RA-05/RA-06).

**K. Dead code.** `machinst_regalloc.rs` (635 LOC), `graph_coloring.rs` (131 LOC), `reg_hint` (dead field), `enable_splitting` (dead field), `handled` (write-only Vec) — ~800 LOC of confusion (P0-01).

## 3. Empirical scoreboard (do not "fix RA" for CRC)

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
10. Do not refactor `immediately_consumed` (RA-23) or `SlotAddr` (RA-24) without full differential testing — they are hard blockers that will miscompile silently.

Kill-switches: `CCC_NO_LOAD_CAST_FOLD`, `CCC_NO_X64_IMMED_NOHOME`, `CCC_MI_FORCE_LOOPS`, `CCC_MI_MAX_LOOP_INSTS`, `CCC_SROA_COPYOUT`, `CCC_EVICT_MODE`, `CCC_NO_COALESCE`, `CCC_DEBUG_RA`, `CCC_DUMP_IR`, `CCC_NO_PHI_COALESCE`, `CCC_NO_LEAF_PARAM_GPR`, `CCC_NO_FOLDED_INDEX_LIVENESS`, `CCC_NO_LOAD_HAZARD_REFINE`, `CCC_NO_EAX_ALLOC`, `CCC_NO_LOOP_PIN`, `CCC_NO_VECREG`, `CCC_PGO_WEIGHT_MAX`, `CCC_TRACE_ALLOC`, `CCC_X64_NOHOME_CLASSES`, `CCC_NO_MACHINST`, `CCC_RA_EXPLAIN` (new, RA-04).

---

## 5. The items

Each row: `ID | P | item | files | evidence | accept | do-not`.

### 5.0 Phase 0 — Hygiene & enablers (do first)

| # | ID | P | Item | Files | Evidence | Accept | Do not |
|---|----|---|------|-------|----------|--------|--------|
| 1 | P0-01 | P0 | **Delete dead code**: `machinst_regalloc.rs` (635 LOC, 0 callers), `graph_coloring.rs` (131 LOC, 0 callers), `reg_hint` field (never `Some`), `enable_splitting` field (never read), `handled` field (write-only) | `regalloc.rs`, `live_range.rs`, `x86/codegen/mod.rs`, `stack_layout/mod.rs` | **C** verified 0 callers | `rg "machinst_regalloc\|color_stack_slots\|reg_hint\|enable_splitting\|\.handled"` returns 0 in `src/backend/` | break tests that construct these fields |
| 2 | P0-02 | P0 | **Switch `live_range.rs` scan from `intervals` to `segments`** | `live_range.rs`, `regalloc.rs` | **C** `segments` exists, consumed for call-span only; scan still fat | xmltok/inflate TU stack-mem drops | change eviction semantics yet |
| 3 | P0-03 | P0 | **Re-measure gzip `longest_match` stack-mem on `b6d0a30`** | `engineering/evidence/` | **C** STATE.md SHA was `3e0ab3e` | number in STATE.md | cite older SHA |
| 4 | P0-04 | P0 | **Wire `alias.rs` into `licm.rs:750`** (replace TODO with `forms_disjoint`) | `licm.rs`, `alias.rs` | **C** TODO at line 750 | more hoists, no alias miscompile | invent a second alias engine |
| 5 | P0-05 | P0 | **Sema type-checking** (was FE-01 P1; reprioritized P0 for correctness — "does NOT reject type errors" is a silent-wrong-code risk) | `sema/analysis.rs` | **C** line 15 TODO; 13 diagnostics vs ~382 in lowerer | incompatible assignments, wrong arg counts, missing returns rejected in sema | silence lowerer checks first |

### 5.1 Register allocation — RA-01 … RA-25

| # | ID | P | Item | Files | Evidence | Accept | Do not |
|---|----|---|------|-------|----------|--------|--------|
| 6 | RA-01 | P0 | Remat file-scope `window`/`prev`/`strstart` as `sym(%rip)` / `sym(%rip,idx,1)` not GOT+stack | `globals.rs`, `live_range.rs`, `generation.rs` | **M** 118 stk; **G** gcc `window(%r9,%rcx)` | match-mini CE: 0 GOT, ≤1 push | PIC/TLS still GOT |
| 7 | RA-02 | P0 | Keep match IVs (`scan`,`best`,`chain`) in GPR | `live_range.rs` next-use | **M** 248 B frame | stack-mem &lt;20 on gzip `longest_match` | eviction mode 5 global |
| 8 | RA-03 | P0 | Adler DO8: evict dead bytes not `sum2`/`n` | `live_range.rs` `future_uses` | **M** 72(%rsp) in loop; **G** icx 0 stk | Adler kernel ≤1.15× GCC | unify incoming/victim cost blindly |
| 9 | RA-04 | P0 | `CCC_RA_EXPLAIN=fn` spill dump | `regalloc.rs` | **C** | one line per spill | dump that changes allocation |
| 10 | RA-05 | P0 | Segment-aware interference (was P1 — `segments` now exists, wire it) | `liveness.rs` segments → scan | **M** xmltok 12× inflate 15× | those ratios &lt;4× | one-home codegen without reloads |
| 11 | RA-06 | P0 | Reload-at-next-use + in-place splitting (was P1 — #1 perf gap, `segments` unblocks) | `live_range.rs`, codegen reload | **C** `live_range.rs:26` "no reload-at-next-use" | gzip unregressed, spill traffic -40% | IR volatile alloca dance only |
| 12 | RA-07 | P1 | Call-site caller-saved save vs always r12 | `regalloc.rs` `caller_save_spans` | **C** Phase 2b slots | use spans more; prologue stk down | promote r11 to callee-saved |
| 13 | RA-08 | P1 | Affinity coalescing overlapping copies | `regalloc.rs` `build_coalesce_groups` | **C** disjoint-only | fewer movs, no extra spills | merge overlapping intervals |
| 14 | RA-09 | P2 | Residual Briggs on spilled | new; **not** `stack_layout/graph_coloring.rs` (deleted in P0-01) | **C** that file colors **slots** | fewer reloads | rename that file as a GPR allocator |
| 15 | RA-10 | P2 | GPR spill to XMM | new class | ICX/ICC | gzip gate | clobber SSE ABI |
| 16 | RA-11 | P1 | One `RangeMetadata` per function | `live_range.rs` | **C** walk × waves | compile time | change eviction semantics |
| 17 | RA-12 | P1 | Set or delete `exchange_eviction` on 2c | `regalloc.rs` vs `live_range.rs` | **C** drift | gzip A/B | default mode 5 |
| 18 | RA-13 | P0 | `verify_no_overlap` → `debug_assert!` (was P1 — non-aborting verification is a correctness risk) | `regalloc.rs:1757` | **C** `eprintln!`-only, O(n²), uses fat `intervals` | debug assert on overlap, abort in debug | ship as silent skip |
| 19 | RA-14 | P2 | Stronger `split_ranges` (phis, multi-exit) — **call-split now works** [DONE note] | `split_ranges.rs` | **C** fail-closed | more splits, gzip gate | mem2reg on split slots |
| 20 | RA-15 | P1 | Param group priority remaining holes | `bump_coalesce_group_priority` | **C** adler/memcmp history | fewer param spills | break SysV arg regs |
| 21 | RA-16 | P2 | Recalibrate PGO RA weights **after** RA-01 | `pgo_weight_max` | **C** +4.7% gzip at 4 | gzip + kernels | raise max first |
| 22 | RA-17 | P1 | Folded GEP **base** liveness x86 remaining | `prologue.rs` ~780 `collect_gep_fold_base_links` | **C** gz_reset NULL-store | no NULL-store miscompile | extend folded **indices** on x86 |
| 23 | RA-18 | P0 | **Delete `machinst_regalloc.rs`** (was P2 "unification" — it's dead code, just delete; merged into P0-01) | `machinst_regalloc.rs` | **C** two scans, zero callers | gone | force MI on gzip |
| 24 | RA-19 | P1 | PhysReg map table in one file | `emit.rs` vs comments | **C** r10/r11 swap | single source of truth | silent renumber |
| 25 | RA-20 | P2 | i686 eax/ecx model → clobber oracle from emit | `regalloc.rs` 2d/2e | **C** | i686 tests | copy x64 policy |
| 26 | RA-21 | P2 | Vecreg whitelist vs new SIMD ops | `collect_vecreg_candidates` | **C** fail-closed | test per intrinsic | open whitelist |
| 27 | RA-22 | P3 | PBQP irregular (AArch64) | — | after RA-01–06 | — | before x64 codecs win |
| 28 | RA-23 | P0 | **Eliminate `immediately_consumed` blocker** — refactor to RA-owned accumulator hint | `stack_layout/copy_coalescing.rs:1328`, `state.rs` | **C** hard-codes accumulator load order (`is_safe_sole_consumer` whitelist) | RA owns accumulator placement; whitelist deleted | break accumulator codegen without differential test |
| 29 | RA-24 | P0 | **Eliminate `SlotAddr::Indirect(StackSlot(0))` dummy** — add `SlotAddr::Reg(PhysReg)` variant | `state.rs:844` | **C** silent corruption if Indirect codepath forgets `reg_assignments` check | exhaustive match catches misses | migrate all backends at once without tests |
| 30 | RA-25 | P1 | **Unify `loop_memory_promote`'s `affine_disjoint` with `alias.rs::forms_disjoint`** | `loop_memory_promote.rs`, `alias.rs` | **C** duplicated SCEV-lite engine | one alias engine | break sqlite `sqlite3FpDecode` fix |

### 5.2 x86 ISel / peephole / MachInst — IS-01 … IS-28

| # | ID | P | Item | Files | Evidence | Accept | Do not |
|---|----|---|------|-------|----------|--------|--------|
| 31 | IS-01 | P0 | CRC `xor table(,%rcx,4), %eax` | `memory.rs` / ALU | **G** gcc; **M** 1.47× 2 vs 0 stk | SIB in crc kernel | emit `crc32` insn (oracles do not at -O2) |
| 32 | IS-02 | P0 | Keep `double` fields in XMM; ban `movq %xmm,%rax` roundtrip | `float_ops.rs`, `memory.rs` | **S** 21× struct_copy | mov count ≤40 | keep rax shuttle |
| 33 | IS-03 | P0 | 64 B assignment → 2× `vmovdqu ymm` | memcpy path **exists**; assignment may not | **G** gcc 7 insns | `copy_s` matches gcc/icx | claim memcpy missing |
| 34 | IS-04 | P0 | Vectorizer emit `vfmadd231pd` into YMM acc **once** horiz | `vectorize.rs` + `intrinsics.rs` | **G** **icx** not gcc16; spectral ICX FMA 10 | `dot`/spectral CE vs icx | copy gcc16 per-iter hadd |
| 35 | IS-05 | P1 | Mem-op `vmulpd (%rdi),…` | ISel fold | **G** | mem operand in vec loop | fold aliased |
| 36 | IS-06 | P1 | `btq` / bitset classify | `simplify.rs` + emit | **G** gcc name_scan; **M** 1.95× | `btq` in name_scan | mega-inline varint |
| 37 | IS-07 | P1 | `cmov` limit idiom | `if_convert.rs` | **G** gcc match `cmovc` | cmov in match | if-convert huge blocks |
| 38 | IS-08 | P1 | Load-cast fold remaining | `prologue.rs` load_cast | **C** 33% gzip-9 was movzbl | leftover movzbl down | disable `CCC_NO_LOAD_CAST_FOLD` globally without measure |
| 39 | IS-09 | P1 | Cmp-replay only from **slots** | keep | **C** sqlite `yy_shift` | no sqlite SEGV | replay from physreg |
| 40 | IS-10 | P1 | Flag fusion Copy/Cast chain | exists; extend carefully | **C** expat accounting SEGV if multi-use | more fused cmp | multi-use flags |
| 41 | IS-11 | P1 | C `__ffs` if-tree → gcc **`andn`+`cmov` chain** (not tzcnt) | `bit_idioms.rs`, `alu.rs` | **G** gcc cmov×11, **not** tzcnt; 1.85× | find_bit CE vs gcc | blindly emit tzcnt |
| 42 | IS-12 | P0 | `a & ~b` → `andn` / `andnq` | ALU BMI2 | **G** find_bit gcc `andn` icx `andnq` | CE find_bit | require tzcnt |
| 43 | IS-13 | P1 | ALU+mem for spilled | peephole | ICX lookalike | fewer reloads | change RA homes |
| 44 | IS-14 | P1 | Dead `movq %r,%r` | peephole | **C** | 0 identity mov | delete live copies |
| 45 | IS-15 | P1 | Redundant `movslq` on indices | narrow vs backend | **C** DOOM −390; rest is GEP | fewer movslq | break GEP signedness |
| 46 | IS-16 | P2 | Match inner 258 B with `pcmpeqb` | vectorize | spike | gzip gate | ship without CRC/adler oracles |
| 47 | IS-17 | P1 | MachInst profit model ≠ 32-inst cutoff | `prologue.rs:812` | **C** gzip −3% | beat gzip then raise | `CCC_MI_FORCE_LOOPS` |
| 48 | IS-18 | P2 | MachInst fused cmp/select | declined | **C** | — | force on gzip |
| 49 | IS-19 | P2 | Text peephole vs MachInst conflict | `CCC_NO_MACHINST` | **C** | documented interaction | dual-apply same fold |
| 50 | IS-20 | P1 | `vzeroupper` only if YMM dirtied | memcpy | **G** gcc emits | AVX transition safe | always vzeroupper |
| 51 | IS-21 | P2 | Scheduler already lost gzip 3% | emit / prologue | **C** `-mtune` diagnostic only | PMU after ISel | turn on as default |
| 52 | IS-22 | P1 | Indexed GEP SIB completeness | generation + memory | **S** loop_patterns 1.85× | SIB in loops | GOT for file-scope |
| 53 | IS-23 | P2 | `repe cmps`/`pcmpeqb` memcmp | | noisy &lt;20 ms | scale work first | optimize 20 ms kernel |
| 54 | IS-24 | P1 | Immediate stores; audit remaining xor+movb | memory | **C** | fewer xor+movb | |
| 55 | IS-25 | P2 | 3-channel mul ILP (`exclude_every_third_mul_temp`) document or kill | `regalloc.rs` | **C** magic | documented or gone | silent heuristic |
| 56 | IS-26 | P1 | GlobalAddr fold vs GOT (`needs_got_for_addr`) | never_materialized | **C** | RIP for file-scope | PIC/TLS |
| 57 | IS-27 | P2 | i128 through rdx:eax without excluding r8–rsi whole fn | prologue i128 filter | **C** | more GPR | ABI break |
| 58 | IS-28 | P0 | Hand-rolled popcount → `UnaryOp::Popcount` (`alu.rs` already emits `popcntl`) | `bit_idioms.rs`, `alu.rs` | **G** bitops gcc/clang `popcntl` | bitops uses insn | add a second popcnt emitter |

### 5.3 Optimizer / alias / SROA / vectorize — OP-01 … OP-33

| # | ID | P | Item | Files | Evidence | Accept | Do not |
|---|----|---|------|-------|----------|--------|--------|
| 59 | OP-01 | P0 | LICM GEP loads via `alias.rs` `forms_disjoint` (merged with P0-04) | `licm.rs:750` | **C** TODO vs existing engine | more hoists, no alias miscompile | invent second alias |
| 60 | OP-02 | P1 | Dominator-aware SROA copy-out | `aggregate_sroa.rs` `CCC_SROA_COPYOUT` | **C** hangs 2 tests | those tests + struct_copy | enable flag without proof |
| 61 | OP-03 | P1 | SROA cross-block memcpy (same-block only now) | same | **C** | more scalarized fields | ignore dominance |
| 62 | OP-04 | P0 | `vectorize_gate` real cost (trip, size, PGO) | `unroll_pgo.rs:122` | **C** always true; **G** clang sieve 365 ymm vs gcc 45 | no 4-iter vec; **no clang-sieve** | copy clang sieve |
| 63 | OP-05 | P0 | Non-reduction FP loop vectorize (nbody YMM) | `vectorize.rs` (~6905) | **S** nbody 3.8× spectral 9.3×; **G** ICX nbody 97 ymm / 58 FMA vs gcc 0 ymm / 24 FMA | ymm+FMA vs ICX | copy gcc16 horiz-per-iter |
| 64 | OP-06 | P0 | Horiz reduce **once** (ICX) not per-iter (gcc16) | vectorize | **G** | one hadd/pack at exit | gcc16 pattern |
| 65 | OP-07 | P1 | Runtime alias versioning | none | **C** | versioned loops, checksums | version everything |
| 66 | OP-08 | P1 | Nested IVSR with dependence | `iv_strength_reduce.rs` skip overlap | **C** nested-matmul wrong if forced | loop_patterns + matmul | force overlapping |
| 67 | OP-09 | P2 | Enable `univsr` after SSA repair | gated | **C** ideas | | enable gated pass raw |
| 68 | OP-10 | P1 | Unroll vs RA pressure | `loop_unroll.rs` | **C** | gzip unregressed | unroll longest_match blindly |
| 69 | OP-11 | P2 | Loop interchange matmul jk | vectorize / loop | **S** | matmul vs ICX | interchange dependent |
| 70 | OP-12 | P1 | Load CSE / GVN less conservative | `gvn.rs` | **C** | fewer redundant loads | CSE aliased |
| 71 | OP-13 | P1 | DCE non-escaping dead stores | `dce.rs` all Store live | **C** | dead stores gone | DCE volatile |
| 72 | OP-14 | P1 | If-convert &gt;8 inst / more diamonds | `if_convert.rs` | **C** | more cmov | convert huge blocks |
| 73 | OP-15 | P2 | `range_check.rs` → `btq` | range_check | wire to IS-06 | btq classify | |
| 74 | OP-16 | P1 | `bit_idioms.rs` coverage (popcnt, andn, ffs) | bit_idioms | **G** | IS-11/12/28 fire | |
| 75 | OP-17 | P2 | `reassoc_accum.rs` vs FMA | reassoc | | FMA-friendly assoc | break fp assoc at -O2 default |
| 76 | OP-18 | P2 | `fp_const_hoist` / `int_const_hoist` audit | | **C** | | |
| 77 | OP-19 | P1 | `load_forward` / `store_load_forward` vs GVN overlap | | compile-time | no double work | |
| 78 | OP-20 | P1 | `loop_memory_promote` more consumers | alias.rs | **C** | more promotions | ignore alias |
| 79 | OP-21 | P2 | `quadratic_sr` | | | | |
| 80 | OP-22 | P1 | `redundant_loads` | | | | |
| 81 | OP-23 | P2 | `block_layout` vs PGO conservative | **M** expat 131→248 | don't reorder | layout reorder hot loop |
| 82 | OP-24 | P2 | `outline_switch` threshold is **40**; measure size vs gcc | `outline_switch.rs:37` | **C** not 999999 | size vs gcc | set 999999 |
| 83 | OP-25 | P1 | Inline `name_cont` | `inline.rs` | **G** gcc inlines | expat scan | 189-inst varint spike |
| 84 | OP-26 | P1 | Pressure-aware inline | inline.rs | sqlite | no +1119 B 1.00× | |
| 85 | OP-27 | P2 | IPCP function cloning | `ipcp.rs` | | | clone explosion |
| 86 | OP-28 | P3 | SCEV full (subsume IVSR+alias+licm) | | high risk | | rewrite all three at once |
| 87 | OP-29 | P1 | Pass order experiment with gzip/adler/expat oracles | `src/passes/mod.rs` | **C** dirty skip | measured order | shuffle without oracles |
| 88 | OP-30 | P2 | Keep `-O0` vs `-O2` distinct | `src/passes/README.md` | **C** tiers real | tests at both | assume all -O identical |
| 89 | OP-31 | P1 | **Unify `loop_memory_promote` alias engine with `alias.rs`** (merged with RA-25) | `loop_memory_promote.rs`, `alias.rs` | **C** 1712 LOC with duplicated SCEV-lite | one alias engine | break sqlite `sqlite3FpDecode` fix |
| 90 | OP-32 | P1 | **Wire `alias.rs` into GVN load CSE** (OP-12) — `forms_disjoint` can prove two loads don't alias | `gvn.rs`, `alias.rs` | **C** GVN is conservative on loads | fewer redundant loads | CSE aliased |
| 91 | OP-33 | P2 | **`segments`-based dead-store elimination** — now that holes are representable, DCE can eliminate stores whose last user is dead | `dce.rs` (currently treats all Store as side-effecting) | **C** `segments` field exists | dead stores gone | DCE volatile |

### 5.4 Frontend / IR — FE-01 … FE-22

| # | ID | P | Item | Files | Evidence | Accept | Do not |
|---|----|---|------|-------|----------|--------|--------|
| 92 | FE-01 | P0 | Sema type diagnostics (not lowerer) — merged with P0-05 | `sema/analysis.rs` | **C** TODO line 15; ~1 vs ~382 | diagnostics in sema | silence lowerer checks first |
| 93 | FE-02 | P1 | TBAA / `restrict` → IR | IR attrs | **C** noalias grep empty | attribute on IR | pretend alias is TBAA |
| 94 | FE-03 | P2 | ExprId not pointer | lowering | **C** TODOs | stable ids | |
| 95 | FE-04 | P2 | Vector subscript | frontend | **C** | | |
| 96 | FE-05 | P2 | typeof diagnostic | | **C** | | |
| 97 | FE-06 | P2 | `__builtin_nan` payload | | **C** | | |
| 98 | FE-07 | P2 | Parser panic-mode | ideas | | | |
| 99 | FE-08 | P2 | `_Complex long double` | | **C** | | |
| 100 | FE-09 | P2 | Unsigned subint IR | `ideas/fix_ir_unsigned_subint*` | **C** | | |
| 101 | FE-10 | P1 | Alignment/range on IR for RA | IR | **C** | better homes | |
| 102 | FE-11 | P2 | `fastcall` fptrs | | **C** | | |
| 103 | FE-12 | P2 | Nested sizeof | | **C** | | |
| 104 | FE-13 | P3 | Use-def shared context | ideas high_use_def | **S** backlog | O(1) use | rewrite every pass at once |
| 105 | FE-14 | P2 | Builtin table unify sema/type_checker | | **C** | | |
| 106 | FE-15 | P2 | C11 UCN lexer | | **C** | | |
| 107 | FE-16 | P1 | Volatile through peephole | prologue `LCCC_VOLATILE_SLOT` | **C** | volatile preserved | fold volatile |
| 108 | FE-17 | P2 | va_arg_pack stub | | **C** | | |
| 109 | FE-18 | P2 | Dependency file `-M` only source | pipeline TODO | **C** | | |
| 110 | FE-19 | P2 | **`__builtin_cpu_supports` runtime CPUID** (was P0 — now FIXED: exact Raptor Lake allowlist; downgrade to P2 for runtime CPUID) | `expr_builtins.rs:436` | **C** FIXED: exact allowlist, not "return 1" | runtime CPUID on non-v3 targets | re-enable the "return 1 for everything" SIGILL bug |
| 111 | FE-20 | P2 | **`usual_arithmetic_conversion` edge cases** (was P0 — now FIXED: correct C11 6.3.1.8 else-arm; downgrade to P2 for edge-case tests) | `common/types.rs:1586` | **C** FIXED: `size > ? signed : unsigned_version` | edge-case regression tests | re-enable the `1LL + 1UL` wrong-type bug |
| 112 | FE-21 | P1 | **`_Pragma` implementation** (currently consumed and dropped) | `macro_defs.rs:542` | **C** C99 §6.10.9 violation | `_Pragma("foo")` injects `#pragma foo` | |
| 113 | FE-22 | P1 | **`__VA_OPT__` support** (C2x) | absent | **C** modern headers use it | `__VA_OPT__` expands | |

### 5.5 ABI / aggregates / stack — AB-01 … AB-15

| # | ID | P | Item | Files | Evidence | Accept | Do not |
|---|----|---|------|-------|----------|--------|--------|
| 114 | AB-01 | P0 | SysV SSE-class return/args in XMM not rax shuttle | ABI + float_ops | **S** struct_copy 21× | XMM class | rax roundtrip |
| 115 | AB-02 | P0 | sret + one wide copy | ABI | same | one ymm/xmm copy | field loop |
| 116 | AB-03 | P1 | 4-byte slots without zext bug | stack_layout README | **C** | | |
| 117 | AB-04 | P1 | Overalign param copy already special-cased; audit | prologue Alignas(32) | **C** | | |
| 118 | AB-05 | P1 | Parallel copy param prestores remaining | prologue gpr_prestores | **C** | fewer cycles | |
| 119 | AB-06 | P2 | RISC-V va_arg long double struct | current_tasks | **C** | | |
| 120 | AB-07 | P2 | i686 double param high word | current_tasks | **C** | | |
| 121 | AB-08 | P2 | ARM KVM VA layout | ideas | **C** | | |
| 122 | AB-09 | P1 | Callee-save + frame compact offsets | peephole | **C** | smaller frames | |
| 123 | AB-10 | P2 | Postgres frame size | ideas | **C** | | |
| 124 | AB-11 | P1 | Alloca escape analysis more GEP | stack_layout | **C** | | |
| 125 | AB-12 | P2 | Two coalescers unify | RA vs stack | **C** | one policy | |
| 126 | AB-13 | P2 | i686 boot 32K | external_tools TODO | **C** | | |
| 127 | AB-14 | P2 | F128 80-bit x86 | ideas | **C** | | |
| 128 | AB-15 | P1 | Dead XMM save in integer varargs remaining | vararg_fp_save | **C** | | |

### 5.6 PGO / IPO / layout — PG-01 … PG-12

| # | ID | P | Item | Files | Evidence | Accept | Do not |
|---|----|---|------|-------|----------|--------|--------|
| 129 | PG-01 | P2 | Sample PGO | ideas high_sample | **C** | | |
| 130 | PG-02 | P2 | Loop-versioned devirt | ideas | **C** | | |
| 131 | PG-03 | P1 | vectorize_gate use profile | OP-04 | **C** | | ignore profile |
| 132 | PG-04 | P1 | RA use block counts after RA-01 | pgo_weight | **C** | gzip gate | `PGO_WEIGHT_MAX>1` first |
| 133 | PG-05 | P2 | Value specialization | ideas | **C** | | |
| 134 | PG-06 | P1 | Keep conservative layout | **M** expat | no 131→248 | reorder hot loop |
| 135 | PG-07 | P2 | LTO | none | | | |
| 136 | PG-08 | P1 | Force-inline budget vs frame | **C** 8B/SSA | | sqlite spike |
| 137 | PG-09 | P2 | Switch profile density vs bt | switch_dispatch 1.40× | **S** | | |
| 138 | PG-10 | P2 | Function placement | | | | |
| 139 | PG-11 | P1 | Flat-profile gate keep | integrated PGO | **M** adler −17% pre-fix | keep `has_spread` | drop gate |
| 140 | PG-12 | P1 | Don't promote 95% single-target | `LCCC_PGO_PROMOTE_STABLE=95` | **M** +28% if promoted | keep | promote stable |

### 5.7 Linker / assembler / multi-arch — LK-01 … LK-18

| # | ID | P | Item | Files | Evidence | Accept | Do not |
|---|----|---|------|-------|----------|--------|--------|
| 141 | LK-01 | P2 | mmap I/O | ideas | | | |
| 142 | LK-02 | P2 | `--icf` | linker | | | |
| 143 | LK-03 | P2 | RISC-V PIC auipc | **C** | | | |
| 144 | LK-04 | P2 | ARM `caspal`, `.org`, PREL64, movw | current_tasks | **C** | | |
| 145 | LK-05 | P2 | RISC-V dash FAIL | current_tasks | **C** | | |
| 146 | LK-06 | P2 | RISC-V kernel boot hang | ideas | **C** | | |
| 147 | LK-07 | P2 | `-mno-sse` full | mod.rs TODO | **C** | | |
| 148 | LK-08 | P2 | Encoder completeness | `encoder/mod.rs` TODO | **C** | | |
| 149 | LK-09 | P2 | i686 archive merge | TODO | **C** | | |
| 150 | LK-10 | P2 | R_X86_64_32 PIC | TODO | **C** | | |
| 151 | LK-11 | P2 | RISC-V `@cc` | TODO | **C** | | |
| 152 | LK-12 | P2 | RVV masked | TODO | **C** | | |
| 153 | LK-13 | P2 | Multi-alt asm ARM/RISCV | TODO | **C** | | |
| 154 | LK-14 | P1 | `encdiff.py` / `insndiff.py` in RA ISel loop | scripts | **C** | | |
| 155 | LK-15 | P2 | ELF32 script setup.elf | ideas | **C** | | |
| 156 | LK-16 | P2 | `--emit-relocs` | | | | |
| 157 | LK-17 | P2 | pcre2 792-nest frame | current_tasks | **C** | | |
| 158 | LK-18 | P3 | QEMU build | ideas | **C** | | |

### 5.8 Measurement / process / compile-time — MS-01 … MS-09

| # | ID | P | Item | Files | Evidence | Accept | Do not |
|---|----|---|------|-------|----------|--------|--------|
| 159 | MS-01 | P0 | Godbolt compare in CI for 6 kernels | `scripts/godbolt.py` | procedure §1 | CI artifact | skip oracles |
| 160 | MS-02 | P0 | Spill-count metric `CCC_TRACE_ALLOCSTATS` aggregate | RA | **C** | number in logs | |
| 161 | MS-03 | P1 | CI correctness+regression+fuzz smoke | ideas README | **C** | | |
| 162 | MS-04 | P1 | OnceLock env knobs break parameterized tests | live_range.rs | **C** | tests independent | |
| 163 | MS-05 | P2 | Compile-time: pass rescans | use-def FE-13 | **C** | | |
| 164 | MS-06 | P1 | Document PhysReg map once | RA-19 | **C** | | |
| 165 | MS-07 | P0 | Re-run Adler/CRC/Expat/gzip `-S` on **current `main`** (merged with P0-03) | workloads | **must** | numbers on this SHA | cite older 118 without re-measure |
| 166 | MS-08 | P1 | **Consolidate 29+ `CCC_` env vars into `RaConfig` struct** | `regalloc.rs` (29), `live_range.rs` (7), `prologue.rs` (16) | **C** 29 in regalloc.rs alone, mixed polarity | `RaConfig` struct, single source of truth | env vars in hot path |
| 167 | MS-09 | P1 | **Audit `peephole_common.rs` UTF-8 fix impact** — old `bytes[i] as char` corrupted non-ASCII bytes in asm (FIXED in `b6d0a30`); verify no compiled binary shipped with corrupted asm | `engineering/evidence/` | **C** old code corrupted UTF-8 | audit report | re-enable `bytes[i] as char` |

---

## 6. First 10 tickets (same as SEQUENCE.md)

1. **P0-01** delete dead code (1 day, zero risk).
2. **MS-07 / P0-03** re-measure `longest_match` stack-mem on current SHA.
3. **RA-04** explain dump.
4. **RA-01 + IS-26** RIP-relative `window`.
5. **RA-02 + RA-03** next-use (Adler + match).
6. **IS-01** CRC SIB.
7. **IS-12 + IS-28** `andn` + popcnt IR.
8. **IS-02 + AB-01** XMM fields / SysV SSE class.
9. **IS-03** 64 B ymm assignment.
10. **IS-04 + OP-05** ICX FMA + non-reduction vec (nbody ymm).

Stop and re-oracle after each. Gzip match stack-mem is the veto.

## 7. File map

```
src/backend/regalloc.rs               3687  RA policy (29 CCC_ env vars)
src/backend/live_range.rs             1261  scan (OnceLock env cache, register_steal_is_safe)
src/backend/liveness.rs               2163  worklist dataflow, segments, gep_base_values
src/backend/split_ranges.rs           1377  IR split (now correct: rewrites uses, phi-safe)
src/backend/x86/codegen/machinst_regalloc.rs  635  DEAD CODE (P0-01)
src/backend/stack_layout/graph_coloring.rs  131  DEAD CODE (P0-01)
src/backend/stack_layout/copy_coalescing.rs 2246  immediately_consumed blocker (RA-23)
src/backend/generation.rs             3622  ISel driver
src/backend/x86/codegen/prologue.rs   1815  RA call, MachInst gate, folds
src/backend/state.rs                   850  SlotAddr::Indirect(StackSlot(0)) dummy (RA-24)
src/backend/peephole_common.rs         408  UTF-8 safe, tab-aware (FIXED)
src/backend/x86/codegen/memory.rs     memcpy ymm
src/backend/x86/codegen/float_ops.rs  FMA scalar
src/backend/x86/codegen/alu.rs        lzcnt/tzcnt/popcnt emitters
src/backend/x86/codegen/machinst*.rs  dual codegen (RA module dead)
src/passes/loop_memory_promote.rs     1712  object_root, memory_on_exit (FIXED); duplicated alias engine (RA-25)
src/passes/alias.rs                    128  LoopFrames, resolve_in_frame, forms_disjoint (underutilized)
src/passes/licm.rs:750                 TODO GEP alias (P0-04)
src/passes/vectorize.rs               6905
src/passes/inline.rs                  3719
src/passes/aggregate_sroa.rs          1009
src/passes/outline_switch.rs:37       MIN_CASES = 40 (FIXED)
src/pgo/unroll_pgo.rs:122             vectorize_gate = true
src/frontend/sema/analysis.rs:15      "does NOT reject type errors" (P0-05)
src/ir/lowering/expr_builtins.rs:436  __builtin_cpu_supports (FIXED: exact allowlist)
src/common/types.rs:1586              usual_arithmetic_conversion (FIXED: C11 else-arm)
scripts/godbolt.py                    CE oracle
engineering/evidence/workloads/
```

## 8. What "beat ICX" means

- **Integer codecs:** remat+RA+SIB (match, Adler, CRC) then ICX XMM-spill if still behind.
- **FP:** copy **ICX** `vfmadd231pd` YMM accumulator, **not** GCC 16.2 per-iter horizontal add.
- **Aggregates:** SysV register class + ymm copies (21× is the largest **S** gap).
- **Parsers:** `btq`/range, segment RA, do **not** mega-inline (varint experiment failed).
- **Bit kernels:** gcc `andn`+`cmov` on find_bit; `popcntl` on bitops; **do not** copy clang sieve ymm.

## 9. What landed since `3e0ab3e` (do not re-fix)

- ✅ **`live_range.rs` Grok variant** — OnceLock env cache, `register_steal_is_safe`, binary search, `run()` idempotency.
- ✅ **`liveness.rs` Grok variant** — worklist dataflow (no `MAX_ITERATIONS` cap), two-latch loop-depth fix, F128 resync, point-level setjmp, GPR-clobber asm.
- ✅ **`split_ranges.rs` Grok variant** — call-split now rewrites uses; loop-split scans all body blocks, phi-safe, span-preserving.
- ✅ **`peephole_common.rs` Grok variant** — UTF-8 safe, tab-aware, `LineStore::replace` no-leak, `replace_source_reg_att`.
- ✅ **`loop_memory_promote.rs` Grok variant** — `object_root` identity, `memory_on_exit` do-while fix, `inst_may_clobber_unknown`, speculation safety.
- ✅ **`outline_switch` MIN_CASES = 40** (was 999999).
- ✅ **`__builtin_cpu_supports` exact allowlist** (was "return 1 for everything").
- ✅ **`usual_arithmetic_conversion` C11 else-arm** (was wrong for `1LL + 1UL`).
- ✅ **`LivenessResult::segments`** — hole-aware intervals, consumed for call-spanning (scan still fat).
- ✅ **`alias.rs`** — `LoopFrames`, `resolve_in_frame`, `forms_disjoint` (consumed by `redundant_loads` only).
