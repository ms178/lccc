# Work backlog

Ranked items for generated-code performance vs GCC 16.2 / Clang 22.1 / ICC 2021.10 / ICX.
Evidence tags: **M** measured LCCC vs GCC14; **G** Compiler Explorer this corpus; **C** code fact; **S** screening suite.

IDs: RA / IS / OP / FE / AB / PG / LK / MS plus IS-ANDN, IS-POPCNT, OP-NO-SIEVE-VEC, …

## 1. How to gather data (mandatory procedure)

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

Godbolt IDs: `cg162`, `cclang2210`, `cicc2021100`, `cicxlatest`.  
**CE 2026-08-20 (this program):** Adler-8 / CRC / match-mini / 64B copy / dot / name_scan — see previous session artifacts if present; re-run on this SHA.

LCCC binary: `scripts/build_lccc_fast.sh` → `target/fastbuild/lccc`. 2 GiB VMs OOM without `scripts/ensure_swap.sh`.

---

## 2. Architectural flaws (read this before coding)

### A. Two codegens, two allocators

- **Text pipeline:** IR → `ArchCodegen` (accumulator `%rax`/`xmm0`) → GNU-as text → **string peephole**.
- **MachInst:** parallel lowering + **second** linear scan (`machinst_regalloc.rs`). Off when loop body >32 insts (gzip).
- **Flaw:** peephole cannot see MachInst; MachInst scheduler already lost gzip. Unifying without a cost model repeats the 3% regression.

### B. Location model is a pile of sets

Homes live in: `reg_assignments`, stack slots, `never_materialized`, `immediately_consumed`, `param_pre_stored`, `fused_cmp_dests`, `cmp_replay`, `load_cast_fold`, XMM vecreg, MachInst regs.  
`ideas/high_value_location_abstraction.txt` Phase1/2 done; **`ValueLocation` enum never landed.** Every new fold adds another HashSet and a SIGSEGV class (documented: load-cast r13, cmp-replay sqlite `yy_shift`, nohome `CCC_X64_NOHOME_CLASSES` excludes `cmp`).

### C. RA scans fat intervals; liveness has holes

`liveness.segments` used for **call-span only**. Scan still `[start,end]`. Diamond CFGs (xmltok/inflate) over-interfere. `split_ranges.rs` is timid IR rewrite, not Wimmer.

### D. Accumulator is still the ISA

Register-direct paths exist for some ALU; FP struct fields still `movq %xmm,%rax; movq %rax,%xmmN` (**struct_copy 21.06×**, 137 vs 29 movs). SysV SSE-class aggregates not kept in XMM across copies.

### E. Sema is a stub relative to lowering

`src/frontend/sema/analysis.rs`: “Full type checking is TODO.” ~1 type diagnostic in sema vs ~382 in lowerer (`ideas/high_sema_expansion_typed_ast.txt`). Opts that need TBAA/`restrict` have **no IR attribute** (`noalias` grep empty).

### F. Alias is loop-SCEV-lite, not TBAA

`alias.rs` wraps `loop_memory_promote` linear forms. LICM **does not call it** for GEP loads. No type-based, no interprocedural points-to.

### G. PGO cannot talk to RA

`CCC_PGO_WEIGHT_MAX` default **1** (gzip +4.7% at 4). Layout **must not reorder** (expat 131→248 ms). `vectorize_gate` ignores profile.

### H. Dual coalescers

RA copy-groups (disjoint intervals) vs `stack_layout/copy_coalescing.rs`. Can disagree.

### I. Text preprocessor + pointer ExprIds

Lowering TODOs: ExprId = pointer; const-eval and typeof diagnostics suffer.

---

## 3. Empirical scoreboard (do not “fix RA” for CRC)

| Workload | LCCC vs GCC14 | Root cause class | Oracle (CE -O2 v3) |
|----------|---------------|------------------|---------------------|
| gzip `longest_match` | 118 vs 0 stack-mem, 248 B frame, GOT | **RA remat + RIP + IV homes** | gcc: `window(%r9,%rcx)`, 1× push rbx |
| zlib-ng Adler kernel | **1.49×** | **RA next-use** (sum2/n on stack in DO8) | icx/clang: 0 stack |
| gzip CRC kernel | **1.49×**, 2 vs 0 spills | **ISel SIB** `crc_table(,%rcx,4)` | gcc 20-line loop |
| Expat name scan | **1.95×** | **classify + inline** | gcc `btq` mask |
| xmltok.c TU | **12×** stack-mem | **RA segments** | — |
| inflate.c TU | **15×** stack-mem | **RA segments + switch** | — |
| `struct_copy` | **21.06×** (585 vs 28 ms) | **ABI + XMM field copies** | gcc/icx: 2× ymm for 64 B `*dst=*src` |
| `dot` | vectorizer reduction exists | **ICX FMA YMM acc**; gcc16.2 worse than ICX | **copy ICX not GCC** |
| nbody/spectral/mandelbrot | **2.9–3.8× / 9.3× S** | **non-reduction vectorize** | — |
| sqlite varint | **~2× S**; 189-inst inline **rejected** (+1119 B, 1.00×) | **ISel branches**, not inline | — |
| linux_find_bit | **1.85× S** | ISel: gcc uses **`andn`+`cmov`** on the C ffs tree, not tzcnt | CE corpus |
| geomean micros | 0.92× | **fib TCE** — ignore for codecs | — |

---

## 4. Hard constraints for the next agent

1. Correctness first: fuzz + `cargo test --lib` + sqlite/expat/gzip if you touch RA/SROA/MachInst.
2. **gzip `longest_match` stack-mem is the RA gate.** Mode 5 eviction and `PGO_WEIGHT_MAX>1` already lost.
3. Do not enable MachInst on large loops without beating that 3% gzip loss.
4. Do not enable `CCC_SROA_COPYOUT` without a dominator proof.
5. PhysReg numbering: **PhysReg(11) is `%r10`**, PhysReg(10) is `%r11` (`prologue.rs` comment).
6. `cmp` must not be in `CCC_X64_NOHOME_CLASSES` (replay vs acc).
7. Paths are `src/…`. Binary is `lccc` not `ccc`.

Kill-switches you will use: `CCC_NO_LOAD_CAST_FOLD`, `CCC_NO_X64_IMMED_NOHOME`, `CCC_MI_FORCE_LOOPS`, `CCC_MI_MAX_LOOP_INSTS`, `CCC_SROA_COPYOUT`, `CCC_EVICT_MODE`, `CCC_NO_COALESCE`, `CCC_DEBUG_RA`, `CCC_DUMP_IR`.

---

## 5. 150 work items by subsystem

**ROI** = expected runtime × #workloads × evidence / (eng-days × risk).  
**P0** = this week if you can land a test. **P1** = this month. **P2** = after P0/P1. **P3** = research.

Each item: `ID | ROI | files | evidence | acceptance | do-not`.

### 5.1 Register allocation (RA-01 … RA-22)

| ID | ROI | Item | Files | Evidence | Accept | Do not |
|----|-----|------|-------|----------|--------|--------|
| RA-01 | P0 | Remat file-scope `window`/`prev`/`strstart` as `sym(%rip)` / `sym(%rip,idx,1)` not GOT+stack | `globals.rs`, `live_range.rs`, `generation.rs` | **M** 118 stk; **G** gcc `window(%r9,%rcx)` | match-mini CE: 0 GOT, ≤1 push | PIC/TLS still GOT |
| RA-02 | P0 | Keep match IVs (`scan`,`best`,`chain`) in GPR | `live_range.rs` next-use | **M** 248 B frame | stack-mem &lt;20 on gzip `longest_match` | mode 5 global |
| RA-03 | P0 | Adler DO8: evict dead bytes not `sum2`/`n` | `live_range.rs` `future_uses` | **M** 72(%rsp) in loop; **G** icx 0 stk | Adler kernel ≤1.15× GCC | unify incoming/victim cost (documented bug) |
| RA-04 | P0 | `CCC_RA_EXPLAIN=fn` dump | `regalloc.rs` | **C** | one line per spill | |
| RA-05 | P1 | Segment-aware interference | `liveness.rs` segments → scan | **M** xmltok 12× inflate 15× | those ratios &lt;4× | one-home codegen without reloads |
| RA-06 | P1 | Second-chance reload / use `enable_splitting` for real | `live_range.rs`, codegen reload | **C** stub | gzip unregressed | IR volatile alloca dance only |
| RA-07 | P1 | Call-site caller-saved save vs always r12 | `regalloc.rs` `caller_save_spans` **already exists** | **C** Phase 2b slots in prologue | use spans more; doProlog stk down | promote r11 to callee-saved (ABI comment) |
| RA-08 | P1 | Affinity coalescing overlapping copies | `regalloc.rs` `build_coalesce_groups` | **C** disjoint-only | fewer movs, no extra spills | merge overlapping intervals |
| RA-09 | P2 | Residual Briggs on spilled | new; not `stack_layout/graph_coloring.rs` | **C** that file is **slots** | | rename file if you touch it |
| RA-10 | P2 | GPR spill to XMM | new class | ICX/ICC | gzip gate | clobber SSE ABI |
| RA-11 | P1 | One `RangeMetadata` per function | `live_range.rs` | **C** walk × waves | compile time | |
| RA-12 | P1 | Set or delete `exchange_eviction` on 2c | `regalloc.rs` vs `live_range.rs` | **C** drift | gzip A/B | default mode 5 |
| RA-13 | P1 | `verify_no_overlap` die-at-birth | `regalloc.rs` | **C** | | |
| RA-14 | P2 | Stronger `split_ranges` (phis, multi-exit) | `split_ranges.rs` | **C** fail-closed | | mem2reg on split slots |
| RA-15 | P1 | Param group priority remaining holes | `bump_coalesce_group_priority` | **C** adler/memcmp history | | |
| RA-16 | P2 | Recalibrate PGO RA weights after RA-01 | `pgo_weight_max` | **C** +4.7% gzip | | raise max first |
| RA-17 | P1 | Folded GEP **base** liveness x86 (already `collect_gep_fold_base_links`) audit remaining | `prologue.rs` ~780 | **C** gz_reset NULL-store | | extend indices on x86 (session-23) |
| RA-18 | P2 | MachInst RA vs IR RA unification | `machinst_regalloc.rs` | **C** two scans | | force MI on gzip |
| RA-19 | P1 | PhysReg map table in one file | `emit.rs` vs comments | **C** r10/r11 swap | | |
| RA-20 | P2 | i686 eax/ecx model → clobber oracle from emit | `regalloc.rs` 2d/2e | **C** | | |
| RA-21 | P2 | Vecreg whitelist vs new SIMD ops | `collect_vecreg_candidates` | **C** fail-closed | test per intrinsic | |
| RA-22 | P3 | PBQP irregular (AArch64) | — | after RA-01–06 | | |

### 5.2 x86 ISel / peephole / MachInst (IS-01 … IS-28)

| ID | ROI | Item | Files | Evidence | Accept |
|----|-----|------|-------|----------|--------|
| IS-01 | P0 | CRC `xor table(,%rcx,4), %eax` | `memory.rs` / ALU | **G** gcc; **M** 1.47× 2 vs 0 stk | SIB in crc kernel |
| IS-02 | P0 | Keep `double` fields in XMM; ban `movq %xmm,%rax` roundtrip | `float_ops.rs`, `memory.rs:1042` | **S** 21× struct_copy | mov count ≤40 |
| IS-03 | P0 | 64 B assignment → 2× `vmovdqu ymm` | memcpy path **exists**; **assignment** may not use it | **G** gcc 7 insns | `copy_s` matches |
| IS-04 | P0 | Vectorizer emit `vfmadd231pd` into YMM acc **once** horiz | `vectorize.rs` + `intrinsics.rs` | **G** **icx** not gcc16 | `dot` CE vs icx |
| IS-05 | P1 | Mem-op `vmulpd (%rdi),…` | ISel fold | **G** | |
| IS-06 | P1 | `btq` / bitset classify | `simplify.rs` + emit | **G** gcc name_scan; **M** 1.95× | |
| IS-07 | P1 | `cmov` limit idiom | if-convert | **G** gcc match `cmovc` | |
| IS-08 | P1 | Load-cast fold remaining (33% gzip-9 cycles **was** longest_match movzbl) | `prologue.rs` load_cast | **C** already; measure leftover | |
| IS-09 | P1 | Cmp-replay only from **slots** (sqlite yy_shift) | keep | **C** | don’t replay from physreg |
| IS-10 | P1 | Flag fusion Copy/Cast chain | exists; extend carefully | **C** expat accounting SEGV if multi-use | |
| IS-11 | P1 | `tzcnt`/`bsf` for find-bit | BMI | **S** 1.85× | |
| IS-12 | P2 | `andn`, `shlx` | BMI2 | | |
| IS-13 | P1 | ALU+mem for spilled | peephole | ICX lookalike | |
| IS-14 | P1 | Dead `movq %r,%r` | peephole | **C** high_codegen | |
| IS-15 | P1 | Redundant `movslq` on indices | narrow vs backend | **C** DOOM −390; rest is GEP | |
| IS-16 | P2 | Match inner 258 B with `pcmpeqb` | | spike | |
| IS-17 | P1 | MachInst profit model ≠ 32-inst cutoff | `prologue.rs:812` | **C** gzip −3% | beat gzip then raise |
| IS-18 | P2 | MachInst fused cmp/select | declined today | **C** | |
| IS-19 | P2 | Text peephole vs MachInst conflict | `CCC_NO_MACHINST` | **C** | |
| IS-20 | P1 | `vzeroupper` only if YMM dirtied | memcpy | **G** gcc emits; measure | |
| IS-21 | P2 | Scheduler: **already lost gzip 3%** | emit.rs:461 | **C** `-mtune` diagnostic only | PMU after ISel |
| IS-22 | P1 | Indexed GEP SIB completeness | generation + memory | **S** loop_patterns 1.85× | |
| IS-23 | P2 | `repe cmps`/`pcmpeqb` memcmp | | noisy &lt;20 ms | scale work first |
| IS-24 | P1 | Immediate stores already; audit remaining xor+movb | | | |
| IS-25 | P2 | 3-channel mul ILP (`exclude_every_third_mul_temp`) document or kill | `regalloc.rs` | **C** magic | |
| IS-26 | P1 | GlobalAddr fold vs GOT (`needs_got_for_addr`) | never_materialized | **C** | |
| IS-27 | P2 | i128 through rdx:eax without excluding r8–rsi whole fn | prologue i128 filter | **C** | |
| IS-28 | P2 | Assembler more encodings (`encoder/mod.rs` TODO) | | compile-time | |

### 5.3 Optimizer / alias / SROA / vectorize (OP-01 … OP-30)

| ID | ROI | Item | Files | Evidence | Accept |
|----|-----|------|-------|----------|--------|
| OP-01 | P0 | LICM GEP loads via `alias.rs` `forms_disjoint` | `licm.rs:750` | **C** TODO vs existing engine | more hoists, no alias miscompile |
| OP-02 | P1 | Dominator-aware SROA copy-out | `aggregate_sroa.rs` CCC_SROA_COPYOUT | **C** hangs 2 tests | those tests + struct_copy |
| OP-03 | P1 | SROA cross-block memcpy (same-block only now) | same | **C** | |
| OP-04 | P0 | `vectorize_gate` real cost (trip, size, PGO) | `unroll_pgo.rs:122` | **C** always true | no 4-iter vec |
| OP-05 | P0 | Non-reduction FP loop vectorize | `vectorize.rs` 6905 lines | **S** nbody 3.8× spectral 9.3× | |
| OP-06 | P0 | Horiz reduce **once** (ICX) not per-iter (gcc16) | vectorize | **G** | |
| OP-07 | P1 | Runtime alias versioning | none | **C** | |
| OP-08 | P1 | Nested IVSR with dependence | `iv_strength_reduce.rs` skip overlap | **C** | |
| OP-09 | P2 | Enable `univsr` after SSA repair | gated | **C** ideas | |
| OP-10 | P1 | Unroll vs RA pressure | `loop_unroll.rs` | **C** | |
| OP-11 | P2 | Loop interchange matmul jk | | **S** | |
| OP-12 | P1 | Load CSE / GVN gen less conservative | `gvn.rs` | **C** | |
| OP-13 | P1 | DCE non-escaping dead stores | `dce.rs` all Store live | **C** | |
| OP-14 | P1 | If-convert &gt;8 inst / more diamonds | `if_convert.rs` | **C** | |
| OP-15 | P2 | `range_check.rs` → `btq` | exists? | wire to IS-06 | |
| OP-16 | P1 | `bit_idioms.rs` coverage | | **G** btq | |
| OP-17 | P2 | `reassoc_accum.rs` vs FMA | | | |
| OP-18 | P2 | `fp_const_hoist` / `int_const_hoist` audit | | | |
| OP-19 | P1 | `load_forward` / `store_load_forward` vs GVN overlap | | compile-time | |
| OP-20 | P1 | `loop_memory_promote` more consumers | alias.rs | | |
| OP-21 | P2 | `quadratic_sr` | | | |
| OP-22 | P1 | `redundant_loads` | | | |
| OP-23 | P2 | `block_layout` vs PGO conservative | **M** expat | don’t reorder | |
| OP-24 | P2 | `outline_switch` threshold is 40; measure size vs gcc | `outline_switch.rs` | | |
| OP-25 | P1 | Inline `name_cont` | `inline.rs` | **G** gcc inlines | |
| OP-26 | P1 | Pressure-aware inline | | sqlite | |
| OP-27 | P2 | IPCP function cloning | `ipcp.rs` | | |
| OP-28 | P3 | SCEV full (subsume IVSR+alias+licm) | | high risk | |
| OP-29 | P1 | Pass order experiment with gzip/adler/expat oracles | `mod.rs` | **C** dirty skip | |
| OP-30 | P2 | `-O0` vs `-O2` still distinct — keep tests at both | | | |

### 5.4 Frontend / IR (FE-01 … FE-18)

| ID | ROI | Item | Evidence |
|----|-----|------|----------|
| FE-01 | P1 | Sema type diagnostics (not lowerer) | **C** analysis.rs TODO |
| FE-02 | P1 | TBAA / `restrict` → IR | noalias grep empty |
| FE-03 | P2 | ExprId not pointer | lowering TODOs |
| FE-04 | P2 | Vector subscript | **C** |
| FE-05 | P2 | typeof diagnostic | **C** |
| FE-06 | P2 | `__builtin_nan` payload | **C** |
| FE-07 | P2 | Parser panic-mode | ideas |
| FE-08 | P2 | `_Complex long double` | **C** |
| FE-09 | P2 | Unsigned subint IR | `fix_ir_unsigned_subint` |
| FE-10 | P1 | Alignment/range on IR for RA | IR audit |
| FE-11 | P2 | `fastcall` fptrs | **C** |
| FE-12 | P2 | Nested sizeof | **C** |
| FE-13 | P3 | Use-def shared context | backlog 10 |
| FE-14 | P2 | Builtin table unify sema/type_checker | **C** |
| FE-15 | P2 | C11 UCN lexer | **C** |
| FE-16 | P1 | Volatile through peephole (markers exist) | prologue LCCC_VOLATILE_SLOT |
| FE-17 | P2 | va_arg_pack stub | **C** |
| FE-18 | P2 | Dependency file `-M` only source | pipeline TODO |

### 5.5 ABI / aggregates / stack (AB-01 … AB-15)

| ID | ROI | Item | Evidence |
|----|-----|------|----------|
| AB-01 | P0 | SysV SSE-class return/args in XMM not rax shuttle | struct_copy 21× |
| AB-02 | P0 | sret + one wide copy | same |
| AB-03 | P1 | 4-byte slots without zext bug | stack_layout README |
| AB-04 | P1 | Overalign param copy already special-cased; audit | prologue Alignas(32) |
| AB-05 | P1 | Parallel copy param prestores (exists) — remaining cycles | prologue gpr_prestores |
| AB-06 | P2 | RISC-V va_arg long double struct | current_tasks |
| AB-07 | P2 | i686 double param high word | current_tasks |
| AB-08 | P2 | ARM KVM VA layout | ideas |
| AB-09 | P1 | Callee-save + frame compact offsets | peephole TODO |
| AB-10 | P2 | Postgres frame size | ideas |
| AB-11 | P1 | Alloca escape analysis more GEP | stack_layout |
| AB-12 | P2 | Two coalescers unify | RA vs stack |
| AB-13 | P2 | i686 boot 32K | external_tools TODO |
| AB-14 | P2 | F128 80-bit x86 | ideas |
| AB-15 | P1 | Dead XMM save in integer varargs (exists) audit remaining | vararg_fp_save |

### 5.6 PGO / IPO / layout (PG-01 … PG-12)

| ID | ROI | Item | Evidence |
|----|-----|------|----------|
| PG-01 | P2 | Sample PGO | backlog 12 |
| PG-02 | P2 | Loop-versioned devirt | backlog 11 |
| PG-03 | P1 | vectorize_gate use profile | OP-04 |
| PG-04 | P1 | RA use block counts after RA-01 | |
| PG-05 | P2 | Value specialization | ideas |
| PG-06 | P1 | Keep conservative layout | **M** expat |
| PG-07 | P2 | LTO | none |
| PG-08 | P1 | Force-inline budget vs frame | **C** 8B/SSA |
| PG-09 | P2 | Switch profile already; density vs bt | switch_dispatch 1.40× |
| PG-10 | P2 | Function placement | |
| PG-11 | P1 | Flat-profile gate keep | integrated |
| PG-12 | P1 | Don’t promote 95% single-target | integrated |

### 5.7 Linker / assembler / multi-arch (LK-01 … LK-18)

| ID | ROI | Item | Evidence |
|----|-----|------|----------|
| LK-01 | P2 | mmap I/O | ideas |
| LK-02 | P2 | `--icf` | |
| LK-03 | P2 | RISC-V PIC auipc | **C** |
| LK-04 | P2 | ARM `caspal`, `.org`, PREL64, movw | current_tasks |
| LK-05 | P2 | RISC-V dash FAIL | current_tasks |
| LK-06 | P2 | RISC-V kernel boot hang | ideas |
| LK-07 | P2 | `-mno-sse` full | mod.rs TODO |
| LK-08 | P2 | Encoder completeness | TODO many insns |
| LK-09 | P2 | i686 archive merge | TODO |
| LK-10 | P2 | R_X86_64_32 PIC | TODO |
| LK-11 | P2 | RISC-V `@cc` | TODO |
| LK-12 | P2 | RVV masked | TODO |
| LK-13 | P2 | Multi-alt asm ARM/RISCV | TODO |
| LK-14 | P1 | `encdiff.py` / `insndiff.py` in RA ISel loop | scripts |
| LK-15 | P2 | ELF32 script setup.elf | ideas |
| LK-16 | P2 | `--emit-relocs` | |
| LK-17 | P2 | pcre2 792-nest frame | current_tasks |
| LK-18 | P3 | QEMU build | ideas |

### 5.8 Measurement / process / compile-time (MS-01 … MS-07)

| ID | ROI | Item | Evidence |
|----|-----|------|----------|
| MS-01 | P0 | Godbolt compare in CI for 6 kernels | procedure §1 |
| MS-02 | P0 | Spill-count metric `CCC_TRACE_ALLOCSTATS` aggregate | |
| MS-03 | P1 | CI correctness+regression+fuzz smoke | ideas README |
| MS-04 | P1 | OnceLock env knobs break parameterized tests | live_range.rs |
| MS-05 | P2 | Compile-time: pass rescans | use-def |
| MS-06 | P1 | Document PhysReg map once | RA-19 |
| MS-07 | P0 | Re-run Adler/CRC/Expat/gzip -S on current `main` | **must** |

---


### Extra ISel tickets (full-corpus CE)

| ID | ROI | Item | Evidence | Accept |
|----|-----|------|----------|--------|
| IS-ANDN | P0 | `a & ~b` → `andn` | find_bit gcc `andn` / icx `andnq` | CE find_bit |
| IS-CMOV-FFS | P1 | C `__ffs` if-tree → gcc `cmov` chain or `tzcnt` if cheaper | gcc cmov×11, **not** tzcnt | 1.85× kernel |
| IS-POPCNT | P0 | Hand-rolled popcount → `UnaryOp::Popcount` (`alu.rs` already emits `popcntl`) | bitops gcc/clang `popcntl` | bitops uses insn |
| OP-NO-SIEVE-VEC | P0 | Do not vectorize sieve fill like clang (365 ins vs gcc 45) | CE sieve | no size explosion |
| OP-NBODY-YMM | P0 | nbody: ICX 97 ymm / 58 FMA vs gcc 0 ymm / 24 FMA | CE | ymm+FMA |
| OP-SPECTRAL-FMA | P0 | spectral: gcc already ymm; ICX adds FMA 10 | CE | vfmadd in vector body |
| RA-ARITH-STK | P1 | arith_loop gcc 107 stack refs | CE | ≤ gcc |

## 6. First 10 tickets for the next agent (sequence)

1. **MS-07** re-measure `longest_match` stack-mem on `0cbdc40` (SROA/MachInst may have moved numbers).
2. **RA-04** explain dump.
3. **RA-01 + IS-26** RIP-relative `window` (CE match-mini vs gcc16.2).
4. **RA-02 + RA-03** next-use (Adler + match).
5. **IS-01** CRC SIB.
6. **IS-02 + AB-01** XMM field copies / SysV SSE class.
7. **IS-03** 64 B `*dst=*src` ymm.
8. **IS-04 + OP-05** ICX-style FMA reduction + non-reduction vec.
9. **IS-06 + OP-25** `btq` + inline classify.
10. **OP-01** LICM←alias.rs.

Stop and re-oracle after each. Gzip match stack-mem is the veto.

## 7. File map (cheat sheet)

```
src/backend/regalloc.rs          3612  RA policy
src/backend/live_range.rs        1261  scan
src/backend/liveness.rs          2163  segments
src/backend/split_ranges.rs      1377  IR split
src/backend/generation.rs        3622  ISel driver
src/backend/x86/codegen/prologue.rs 1814  RA call, MachInst gate, folds
src/backend/x86/codegen/memory.rs    memcpy ymm
src/backend/x86/codegen/float_ops.rs FMA scalar
src/backend/x86/codegen/machinst*.rs dual codegen
src/passes/vectorize.rs          6905
src/passes/inline.rs             3719
src/passes/aggregate_sroa.rs     1010
src/passes/alias.rs              loop SCEV-lite
src/passes/licm.rs:750           TODO GEP alias
src/pgo/unroll_pgo.rs:122        vectorize_gate = true
scripts/godbolt.py               CE oracle
engineering/evidence/workloads/gzip-zlib-expat.md
engineering/evidence/workloads/struct-copy.md
```

## 8. What “beat ICX” means here

- **Integer codecs:** remat+RA+SIB (match, Adler, CRC) then ICX XMM-spill if still behind.
- **FP:** copy **ICX** `vfmadd231pd` YMM accumulator, **not** GCC 16.2’s per-iter horizontal.
- **Aggregates:** SysV register class + ymm copies (21× is the largest **S** gap).
- **Parsers:** `btq`/range, segment RA, do **not** mega-inline (varint experiment failed).

End of briefing.
