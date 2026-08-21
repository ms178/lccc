# 150 highest-ROI work items

Ranked work for **generated-code performance** vs GCC 16.2 / Clang 22.1 / ICC 2021.10 / ICX.
This file **is** the 150-item catalog (IDs P0 / RA / IS / OP / FE / AB / PG / LK / MS). Do not replace it with a stub.

Evidence: **M** measured LCCC vs GCC14 on kernels; **G** Compiler Explorer (`cg162` / `cclang2210` / `cicc2021100` / `cicxlatest`, `-O2 -march=x86-64-v3`); **C** tree fact (line numbers verified at `35a6b88`); **S** screening suite.

**ROI rank:** P0 this week (testable) · P1 this month · P2 after P0/P1 · P3 research.

Count: P0 5 + RA 27 + IS 28 + OP 34 + FE 22 + AB 15 + PG 12 + LK 18 + MS 9 = **170** (extra items are refinements; drop `[NEW]`/`[REFINED]` tags when stable).

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

**A. Two codegens, one dead allocator.** Text IR → GNU-as + string peephole, and MachInst ISel/emit + a **dead** second linear scan (`machinst_regalloc.rs`, 635 LOC, zero callers, RAX-clobber soundness bug in `rewrite_machinsts`). The MachInst ISel/emit path is gated off when loop body >32 insts (gzip −3%). Deleting the dead RA (P0-01a) is zero-risk. The MachInst ISel/emit path piggy-backs on the main RA via `resolve_stack_vregs` (`emit.rs:2998`) — it does not need its own RA. Unifying the ISel paths without a cost model repeats the gzip regression.

**B. Location model is converging.** `ExplicitLocation::{Reg,Accumulator}` now owns non-stack scalar homes; `SlotAddr::Reg` is production across all backends. Remaining debt is moving accumulator legality from the stack-layout whitelist into RA, plus `never_materialized`, param prestores, compare replay, and vector/MachInst locations.

**C. RA scans fat intervals.** `liveness.segments` (hole-aware) is consumed by `regalloc.rs` for call-spanning detection and interval extension (lines 571, 785), but the linear scan itself (`live_range.rs`) still runs on fat `intervals` (single `[start,end]`). Diamond CFGs (xmltok/inflate) over-interfere. `split_ranges.rs` is now a correct IR rewrite (rewrites uses, phi-safe), but is not Wimmer-style in-place splitting.

**D. Accumulator is still the ISA.** FP struct fields still `movq %xmm,%rax; movq %rax,%xmmN` (`struct_copy` 21.06×). `immediately_consumed` hard-codes the accumulator load order (RA-23).

**E. Sema now enforces core constraints.** Assignment, return, and fixed-prototype arity diagnostics run before lowering (P0-05/FE-01). Remaining frontend type work is canonical anonymous aggregate identity and broader operator constraints. IR already preserves pointer `noalias`; broader TBAA remains open.

**F. Alias is loop-SCEV-lite, underutilized.** `alias.rs` (128 LOC) has `LoopFrames`, `resolve_in_frame`, `forms_disjoint`. Consumed by `redundant_loads` only. LICM does **not** call it for GEP loads (`licm.rs:750` TODO). `loop_memory_promote.rs` (1712 LOC) has its own duplicated `affine_disjoint` engine (RA-25 unifies).

**G. PGO remains conservative.** `CCC_PGO_WEIGHT_MAX` default 1 (gzip +4.7% at 4), and layout must not reorder (expat 131→248 ms). Vectorization now consumes exact per-loop trip/body cost; RA profile weighting remains intentionally capped.

**H. Dual coalescers.** RA copy-groups vs `stack_layout/copy_coalescing.rs`.

**I. ExprId is a pointer.** Lowering TODOs; const-eval / typeof suffer.

**J. Lifetime demotion is the only spill model.** `live_range.rs:26-27`: "There is no reload-at-next-use in this module." 3-5× spill traffic vs LLVM. `segments` infrastructure is built but the scan doesn't use it for splitting (RA-05/RA-06).

**K. Dead code — nuanced.** Not all dead code should be deleted. `machinst_regalloc.rs` (635 LOC, 0 callers, RAX-clobber soundness bug) — **DELETE** (P0-01a). `graph_coloring.rs` (131 LOC, sound algorithm) — **KEEP**, wire into Tier 2 after RA-23 (P0-01b). `reg_hint` field — **WIRE UP** for ABI-mandated registers (RA-26). `enable_splitting` field — **KEEP** as RA-06 stub (P0-01d). `handled` field — **WIRE INTO** RA-13 verification (P0-01e). See STATE.md "Dead code — nuanced assessment" for full rationale.

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
10. Do not refactor `immediately_consumed` (RA-23) without full differential testing. Preserve RA-24's exact `SlotAddr::Reg(PhysReg)` contract.
11. Do not delete `graph_coloring.rs` — it is sound infrastructure blocked by RA-23 (wire after P0-01b).
12. Do not delete `reg_hint`/`enable_splitting`/`handled` fields — wire them up (RA-26/RA-06/RA-13).

Kill-switches: `CCC_NO_LOAD_CAST_FOLD`, `CCC_NO_X64_IMMED_NOHOME`, `CCC_MI_FORCE_LOOPS`, `CCC_MI_MAX_LOOP_INSTS`, `CCC_SROA_COPYOUT`, `CCC_EVICT_MODE`, `CCC_NO_COALESCE`, `CCC_DEBUG_RA`, `CCC_DUMP_IR`, `CCC_NO_PHI_COALESCE`, `CCC_NO_LEAF_PARAM_GPR`, `CCC_NO_FOLDED_INDEX_LIVENESS`, `CCC_NO_LOAD_HAZARD_REFINE`, `CCC_NO_EAX_ALLOC`, `CCC_NO_SEGMENT_FILL`, `CCC_NO_INDEX_HOME`, `CCC_NO_ABI_REG_HINTS`, `CCC_NO_LICM_ALIAS`, `CCC_NO_ANDN_FUSION`, `CCC_NO_64B_YMM_COPY`, `CCC_ENABLE_TIER2_GRAPH`, `CCC_NO_LOOP_PIN`, `CCC_NO_VECREG`, `CCC_PGO_WEIGHT_MAX`, `CCC_TRACE_ALLOC`, `CCC_X64_NOHOME_CLASSES`, `CCC_NO_MACHINST`, `CCC_RA_EXPLAIN`.

---

## 5. The items

Each row: `ID | P | item | files | evidence | accept | do-not`.

### 5.0 Phase 0 — Hygiene & enablers (do first)

| # | ID | P | Item | Files | Evidence | Accept | Do not |
|---|----|---|------|-------|----------|--------|--------|
| 1 | P0-01a | DONE | Deleted only `machinst_regalloc.rs` (635 LOC, zero callers, RAX spill-clobber bug); live MachInst ISel/emit remains on main RA | module + declaration | **C** zero-reference audit | 962 unit tests | delete live MachInst path or graph colorer |
| 2 | P0-01b | DONE | Hole-aware exact-size Tier-2 coloring is production default; copy/phi/multi-def/asm webs are quarantined, definition writes occupy closed points, and `CCC_NO_TIER2_GRAPH` is the kill switch | graph colorer + Tier 2 | **M** huft frame 1848→1832; huft/SQLite/MachInst runtime; 382 regression + 600 phi + 540 alias | default enabled with conservative unsafe partition | color unresolved alias webs |
| 3 | P0-02 | PARTIAL | i686 residual segment coloring now fills holes in already-saved callee regs without eviction/new saves; full split scan remains RA-05/06 | `regalloc.rs` | **M** boot C text -1,573 B, stack refs -13.1% | full xmltok/inflate treatment after RA-23 | pretend residual fill is reload-at-next-use |
| 4 | P0-03 | DONE | Re-measured pinned gzip 1.14 on `50e7ac83` + patch | workload artifacts | **M** 30/30 tests; longest_match LCCC 303 insn/119 stack-mem vs GCC 114/0; treatment equals kill-switch control | numbers recorded in session doc | cite old 118 |
| 5 | P0-04 | DONE | LICM now uses shared `LinForm`/`forms_disjoint`; resolver gains checked Shl scaling; calls/atomics/unresolved stores fail closed | `licm.rs`, `loop_memory_promote.rs`, `alias.rs` | **M** invariant `a[0]` hoisted across marching `a[i+1]` store; 720 alias + 600 phi cases clean | targeted regression | invent second engine |
| 6 | P0-05 | DONE | Sema now rejects named-aggregate and incompatible-pointer assignments, direct/indirect prototype arity errors, and invalid valued/valueless returns; unspecified legacy prototypes remain permissive | `sema/analysis.rs` | **M** six-error negative corpus plus valid variadic/void*/function-pointer control; 50/50 correctness, 364 runnable regressions | diagnostics originate in semantic analysis | reject anonymous SIMD typedefs before type identity is canonical |

### 5.1 Register allocation — RA-01 … RA-27

| # | ID | P | Item | Files | Evidence | Accept | Do not |
|---|----|---|------|-------|----------|--------|--------|
| 7 | RA-01 | P0 | Remat file-scope `window`/`prev`/`strstart` as `sym(%rip)` / `sym(%rip,idx,1)` not GOT+stack | `globals.rs`, `live_range.rs`, `generation.rs` | **M** 118 stk; **G** gcc `window(%r9,%rcx)` | match-mini CE: 0 GOT, ≤1 push | PIC/TLS still GOT |
| 8 | RA-02 | P0 | Keep match IVs (`scan`,`best`,`chain`) in GPR | `live_range.rs` next-use | **M** 248 B frame | stack-mem <20 on gzip `longest_match` | eviction mode 5 global |
| 9 | RA-03 | P0 | Adler DO8: evict dead bytes not `sum2`/`n` | `live_range.rs` `future_uses` | **M** 72(%rsp) in loop; **G** icx 0 stk | Adler kernel ≤1.15× GCC | unify incoming/victim cost blindly |
| 10 | RA-04 | DONE | `CCC_RA_EXPLAIN=fn` deterministic spill report: range, segments, weighted uses, reason class | `regalloc.rs` | **C/M** used on current gzip | one line per spill | dump that changes allocation |
| 11 | RA-05 | P0 | Segment-aware interference (was P1 — `segments` now exists, wire it) | `liveness.rs` segments → scan | **M** xmltok 12× inflate 15× | those ratios <4× | one-home codegen without reloads |
| 12 | RA-06 | P0 | Reload-at-next-use + in-place splitting (was P1 — #1 perf gap, `segments` unblocks). Gate on `enable_splitting` field (P0-01d — keep, don't delete) | `live_range.rs`, codegen reload | **C** `live_range.rs:26` "no reload-at-next-use" | gzip unregressed, spill traffic -40% | IR volatile alloca dance only |
| 13 | RA-07 | P1 | Call-site caller-saved save vs always r12 | `regalloc.rs` `caller_save_spans` | **C** Phase 2b slots | use spans more; prologue stk down | promote r11 to callee-saved |
| 14 | RA-08 | P1 | Affinity coalescing overlapping copies | `regalloc.rs` `build_coalesce_groups` | **C** disjoint-only | fewer movs, no extra spills | merge overlapping intervals |
| 15 | RA-09 | P2 | Residual Briggs on spilled | new; **not** `stack_layout/graph_coloring.rs` (KEEP — P0-01b) | **C** that file colors **slots** (sound, blocked on RA-23) | fewer reloads | rename that file as a GPR allocator |
| 16 | RA-10 | P2 | GPR spill to XMM | new class | ICX/ICC | gzip gate | clobber SSE ABI |
| 17 | RA-11 | P1 | One `RangeMetadata` per function | `live_range.rs` | **C** walk × waves | compile time | change eviction semantics |
| 18 | RA-12 | DONE | Deleted dead `exchange_eviction` policy bit (zero writers); eviction default is explicitly mode 3 and `CCC_EVICT_MODE=5` remains opt-in only | `live_range.rs` | **C** zero-writer audit; removes false phase-2c contract | full suites + gzip | silently default mode 5 |
| 19 | RA-13 | DONE | Hard hole-aware final verifier plus `handled` occupancy history (`phys_reg`, eviction cut point), O(n log n) | `regalloc.rs`, `live_range.rs` | **C** old print-only verifier missed evicted history | full suites under `CCC_VERIFY_REGALLOC` | ship silent warning |
| 20 | RA-14 | P2 | Stronger `split_ranges` (phis, multi-exit) — **call-split now works** [DONE] | `split_ranges.rs` | **C** fail-closed | more splits, gzip gate | mem2reg on split slots |
| 21 | RA-15 | P1 | Param group priority remaining holes | `bump_coalesce_group_priority` | **C** adler/memcmp history | fewer param spills | break SysV arg regs |
| 22 | RA-16 | P2 | Recalibrate PGO RA weights **after** RA-01 | `pgo_weight_max` | **C** +4.7% gzip at 4 | gzip + kernels | raise max first |
| 23 | RA-17 | P1 | Folded GEP **base** liveness x86 remaining | `prologue.rs` ~780 `collect_gep_fold_base_links` | **C** gz_reset NULL-store | no NULL-store miscompile | extend folded **indices** on x86 |
| 24 | RA-19 | P1 | PhysReg map table in one file | `emit.rs` vs comments | **C** r10/r11 swap | single source of truth | silent renumber |
| 25 | RA-20 | P2 | i686 eax/ecx model → clobber oracle from emit | `regalloc.rs` 2d/2e | **C** | i686 tests | copy x64 policy |
| 26 | RA-21 | P2 | Vecreg whitelist vs new SIMD ops | `collect_vecreg_candidates` | **C** fail-closed | test per intrinsic | open whitelist |
| 27 | RA-22 | P3 | PBQP irregular (AArch64) | — | after RA-01–06 | — | before x64 codecs win |
| 28 | RA-23 | DONE | RA owns target-policy accumulator assignments with verified def/consume points and physical-home precedence; all backends consume allocator results; MachInst enforces point lifetimes; Tier-2 definition stores and alias-sensitive webs are modeled/quarantined | RA + state + MachInst + stack layout | **M/C** 382 regression, 600 phi, 540 alias; huft/SQLite fixed; Tier-2 default | hard verifier + kill switches | infer location from missing slots |
| 29 | RA-24 | DONE | Replaced fake `Indirect(StackSlot(0))` with exact `SlotAddr::Reg(PhysReg)`; migrated shared memory/GEP/memcpy, soft-F128, x86/x87, i686, AArch64, and RISC-V consumers | state + 18 backend files | **M/C** compiler-enforced 51-site migration; native mixed-width runtime; all-target scalar/F128 compile; 985 unit + 378 regression + 600 phi + 540 alias | exact physical home consumed | reintroduce dummy frame offsets |
| 30 | RA-25 | DONE | Loop memory promotion delegates separation to shared `alias::forms_disjoint`; shared arithmetic is checked for const/march/end overflow | LMP + alias | **C/M** duplicate affine math removed; overflow fails closed; loop/alias suites | one checked alias engine | restore unchecked subtraction |
| 31 | RA-26 | DONE | ABI physical hints plus ordered caller-home placement now cover scalar call-free x86 CFG leaves with ≤6 register arguments and leading entry ParamRefs; stack-argument/mixed/calling shapes fail closed | RA + x86 prologue | **G/M** branch leaf 16→8 body instructions vs control; CE LCCC 10 vs GCC/ICC 8, Clang/ICX 9; 377 regressions + 50 correctness | `CCC_NO_LEAF_PARAM_GPR` | admit stack args or late ParamRefs |
| 32 | RA-27 | DONE | `-O0` non-SSA multi-def IR uses canonical stack homes on all backends; production RA remains enabled at O1/O2/O3/Os/Oz | CodegenOptions + all prologues | **M** phi CFG improved 475/600 → 600/600; seed-1 stale loop-carried value fixed; cross-target O0 compile | 600 phi + 540 alias + dedicated runtime | run one-def linear scan on multi-def IR |

### 5.2 x86 ISel / peephole / MachInst — IS-01 … IS-28

| # | ID | P | Item | Files | Evidence | Accept | Do not |
|---|----|---|------|-------|----------|--------|--------|
| 32 | IS-01 | DONE | Masked byte-table indexes retain a safe hidden home; variable-index GlobalAddrs remain site-local through GlobalAddr CSE/GVN so x86 still emits scale-4 SIB loads | RA + GAddr CSE/GVN + indexed emitters | **M** repaired PR161 regression (`movl (%rsi,%rdx,4),%edi`); CRC runtime + structural gate; 600 phi | SIB regression | entry-hoist indexed symbol bases |
| 33 | IS-02 | P0 | Keep `double` fields in XMM; ban `movq %xmm,%rax` roundtrip | `float_ops.rs`, `memory.rs` | **S** 21× struct_copy | mov count ≤40 | keep rax shuttle |
| 34 | IS-03 | DONE | AVX2 64-byte assignment → 2 YMM load/store pairs + `vzeroupper`; safe leaf DCE removes dead parameter homes and the entire frame/setup | x86 memcpy + target feature contract + DCE | **G/M** 6 instructions, exact parity with GCC16.2/Clang22.1/ICX latest (ICC 9); runtime exact; baseline ISA remains XMM | structural/runtime regression | use YMM for 32/48 B (measured slower) |
| 35 | IS-04 | P0 | Vectorizer emit `vfmadd231pd` into YMM acc **once** horiz | `vectorize.rs` + `intrinsics.rs` | **G** **icx** not gcc16; spectral ICX FMA 10 | `dot`/spectral CE vs icx | copy gcc16 per-iter hadd |
| 36 | IS-05 | P1 | Mem-op `vmulpd (%rdi),…` | ISel fold | **G** | mem operand in vec loop | fold aliased |
| 37 | IS-06 | DONE (2026-08-21 session 38) | Cross-target BitTest canonicalization | `ir/ops.rs`, `simplify.rs`, all backends | **G/M** `(x >> i) & 1` becomes `BitTest`; x86 uses BT, AArch64 UBFX for constant indexes, RISC-V/i686 use shift/AND | `check_bit_test_canonical.sh` |
| 38 | IS-07 | P1 | `cmov` limit idiom | `if_convert.rs` | **G** gcc match `cmovc` | cmov in match | if-convert huge blocks |
| 39 | IS-08 | P1 | Load-cast fold remaining | `prologue.rs` load_cast | **C** 33% gzip-9 was movzbl | leftover movzbl down | disable `CCC_NO_LOAD_CAST_FOLD` globally without measure |
| 40 | IS-09 | P1 | Cmp-replay only from **slots** | keep | **C** sqlite `yy_shift` | no sqlite SEGV | replay from physreg |
| 41 | IS-10 | P1 | Flag fusion Copy/Cast chain | exists; extend carefully | **C** expat accounting SEGV if multi-use | more fused cmp | multi-use flags |
| 42 | IS-11 | P1 | C `__ffs` if-tree → gcc **`andn`+`cmov` chain** (not tzcnt) | `bit_idioms.rs`, `alu.rs` | **G** gcc cmov×11, **not** tzcnt; 1.85× | find_bit CE vs gcc | blindly emit tzcnt |
| 43 | IS-12 | DONE | Adjacent single-use `not`+`and` → BMI1 `andn{l,q}`, direct physical sources, baseline ISA gated | CodegenOptions + shared fusion + x86 ALU | **M** linux_find_bit 1.04x faster vs control, 1.42x behind GCC | BMI/no-BMI runtime+assembly regression | emit without BMI contract |
| 44 | IS-13 | P1 | ALU+mem for spilled | peephole | ICX lookalike | fewer reloads | change RA homes |
| 45 | IS-14 | DONE (2026-08-21 session 37) | 32-bit integer returns now use `operand_to_rax_i32`; self-moves remain covered by the existing `SelfMove` peephole | `returns.rs`, `emit.rs`, `check_return_32bit_zeroext.sh` | **C/M** leaf boolean/classifier returns use `movl`/`xorl`, not `movq` | structural regression | delete live copies |
| 46 | IS-15 | P1 | Redundant `movslq` on indices | narrow vs backend | **C** DOOM −390; rest is GEP | fewer movslq | break GEP signedness |
| 47 | IS-16 | P2 | Match inner 258 B with `pcmpeqb` | vectorize | spike | gzip gate | ship without CRC/adler oracles |
| 48 | IS-17 | P1 | MachInst profit model ≠ 32-inst cutoff | `prologue.rs:812` | **C** gzip −3% | beat gzip then raise | `CCC_MI_FORCE_LOOPS` |
| 49 | IS-18 | P2 | MachInst fused cmp/select | declined | **C** | — | force on gzip |
| 50 | IS-19 | P2 | Text peephole vs MachInst conflict | `CCC_NO_MACHINST` | **C** | documented interaction | dual-apply same fold |
| 51 | IS-20 | P1 | `vzeroupper` only if YMM dirtied | memcpy | **G** gcc emits | AVX transition safe | always vzeroupper |
| 52 | IS-21 | P2 | Scheduler already lost gzip 3% | emit / prologue | **C** `-mtune` diagnostic only | PMU after ISel | turn on as default |
| 53 | IS-22 | P1 | Indexed GEP SIB completeness | generation + memory | **S** loop_patterns 1.85× | SIB in loops | GOT for file-scope |
| 54 | IS-23 | P2 | `repe cmps`/`pcmpeqb` memcmp | | noisy <20 ms | scale work first | optimize 20 ms kernel |
| 55 | IS-24 | P1 | Immediate stores; audit remaining xor+movb | memory | **C** | fewer xor+movb | |
| 56 | IS-25 | P2 | 3-channel mul ILP (`exclude_every_third_mul_temp`) document or kill | `regalloc.rs` | **C** magic | documented or gone | silent heuristic |
| 57 | IS-26 | P1 | GlobalAddr fold vs GOT (`needs_got_for_addr`) | never_materialized | **C** | RIP for file-scope | PIC/TLS |
| 58 | IS-27 | P2 | i128 through rdx:eax without excluding r8–rsi whole fn | prologue i128 filter | **C** | more GPR | ABI break |
| 59 | IS-28 | DONE (pre-existing, revalidated) | Canonical SWAR popcount recognized in `bit_idioms` → `UnaryOp::Popcount`; x86 emits `popcntl` under v3 | optimizer + existing emitter | **G/M** current bitops assembly contains popcntl | unit near-miss + benchmark oracle | add duplicate emitter |

### 5.3 Optimizer / alias / SROA / vectorize — OP-01 … OP-34

| # | ID | P | Item | Files | Evidence | Accept | Do not |
|---|----|---|------|-------|----------|--------|--------|
| 60 | OP-01 | DONE | Shared linear-form alias proof wired into LICM with fail-closed store modeling | `licm.rs`, `alias.rs`, `loop_memory_promote.rs` | **M** targeted hoist; full differential clean | regression | model unknown writes |
| 61 | OP-02 | BLOCKED | Dominator-aware SROA copy-out remains default-off | aggregate_sroa | **M** current flag does not transform struct_copy (+2.2% noise) and miscompiles `structs_bitfields` (return 15); attempted loop relaxation reverted | require dominance/ABI proof + full fuzz | enable raw flag |
| 62 | OP-03 | P1 | SROA cross-block memcpy (same-block only now) | same | **C** | more scalarized fields | ignore dominance |
| 63 | OP-04 | DONE | Active vectorizer now asks a per-natural-loop PGO cost gate; exact trips <8 are rejected and bodies >80 instructions require ≥32 trips; unprofiled builds retain static policy | `unroll_pgo.rs`, `vectorize.rs` | **C/M** dead function-only gate removed; threshold unit tests; existing constant-trip regression retained | no short profiled vectorization; no function-wide switch | force-enable on profile heat alone |
| 64 | OP-05 | P0 | Non-reduction FP loop vectorize (nbody YMM) | `vectorize.rs` (~6905) | **S** nbody 3.8× spectral 9.3×; **G** ICX nbody 97 ymm / 58 FMA vs gcc 0 ymm / 24 FMA | ymm+FMA vs ICX | copy gcc16 horiz-per-iter |
| 65 | OP-06 | P0 | Horiz reduce **once** (ICX) not per-iter (gcc16) | vectorize | **G** | one hadd/pack at exit | gcc16 pattern |
| 66 | OP-07 | P1 | Runtime alias versioning | none | **C** | versioned loops, checksums | version everything |
| 67 | OP-08 | P1 | Nested IVSR with dependence | `iv_strength_reduce.rs` skip overlap | **C** nested-matmul wrong if forced | loop_patterns + matmul | force overlapping |
| 68 | OP-09 | P2 | Enable `univsr` after SSA repair | gated | **C** ideas | | enable gated pass raw |
| 69 | OP-10 | P1 | Unroll vs RA pressure | `loop_unroll.rs` | **C** | gzip unregressed | unroll longest_match blindly |
| 70 | OP-11 | P2 | Loop interchange matmul jk | vectorize / loop | **S** | matmul vs ICX | interchange dependent |
| 71 | OP-12 | P1 | Load CSE / GVN less conservative | `gvn.rs` | **C** | fewer redundant loads | CSE aliased |
| 72 | OP-13 | P1 | DCE non-escaping dead stores | `dce.rs` all Store live | **C** | dead stores gone | DCE volatile |
| 73 | OP-14 | P1 | If-convert >8 inst / more diamonds | `if_convert.rs` | **C** | more cmov | convert huge blocks |
| 74 | OP-15 | P2 | `range_check.rs` → `btq` | range_check | wire to IS-06 | btq classify | |
| 75 | OP-16 | P1 | `bit_idioms.rs` coverage (popcnt, andn, ffs) | bit_idioms | **G** | IS-11/12/28 fire | |
| 76 | OP-17 | P2 | `reassoc_accum.rs` vs FMA | reassoc | | FMA-friendly assoc | break fp assoc at -O2 default |
| 77 | OP-18 | P2 | `fp_const_hoist` / `int_const_hoist` audit | | **C** | | |
| 78 | OP-19 | P1 | `load_forward` / `store_load_forward` vs GVN overlap | | compile-time | no double work | |
| 79 | OP-20 | P1 | `loop_memory_promote` more consumers | alias.rs | **C** | more promotions | ignore alias |
| 80 | OP-21 | P2 | `quadratic_sr` | | | | |
| 81 | OP-22 | P1 | `redundant_loads` | | | | |
| 82 | OP-23 | P2 | `block_layout` vs PGO conservative | **M** expat 131→248 | don't reorder | layout reorder hot loop |
| 83 | OP-24 | P2 | `outline_switch` threshold is **40**; measure size vs gcc | `outline_switch.rs:37` | **C** not 999999 | size vs gcc | set 999999 |
| 84 | OP-25 | P1 | Inline `name_cont` | `inline.rs` | **G** gcc inlines | expat scan | 189-inst varint spike |
| 85 | OP-26 | P1 | Pressure-aware inline | inline.rs | sqlite | no +1119 B 1.00× | |
| 86 | OP-27 | P2 | IPCP function cloning | `ipcp.rs` | | | clone explosion |
| 87 | OP-28 | P3 | SCEV full (subsume IVSR+alias+licm) | | high risk | | rewrite all three at once |
| 88 | OP-29 | P1 | Pass order experiment with gzip/adler/expat oracles | `src/passes/mod.rs` | **C** dirty skip | measured order | shuffle without oracles |
| 89 | OP-30 | P2 | Keep `-O0` vs `-O2` distinct | `src/passes/README.md` | **C** tiers real | tests at both | assume all -O identical |
| 90 | OP-31 | P1 | **Unify `loop_memory_promote` alias engine with `alias.rs`** (merged with RA-25) | `loop_memory_promote.rs`, `alias.rs` | **C** 1712 LOC with duplicated SCEV-lite | one alias engine | break sqlite `sqlite3FpDecode` fix |
| 91 | OP-32 | P1 | **Wire `alias.rs` into GVN load CSE** (OP-12) — `forms_disjoint` can prove two loads don't alias | `gvn.rs`, `alias.rs` | **C** GVN is conservative on loads | fewer redundant loads | CSE aliased |
| 92 | OP-33 | P2 | **`segments`-based dead-store elimination** — now that holes are representable, DCE can eliminate stores whose last user is dead | `dce.rs` (currently treats all Store as side-effecting) | **C** `segments` field exists | dead stores gone | DCE volatile |
| 92a | OP-34 | DONE | GlobalAddr CSE uses oracle-derived placement: cold singleton stays local; guarded loop moves to innermost preheader; dominating existing defs merge; mutually-exclusive siblings stay replicated; derived variable-index bases remain site-local | `global_addr_cse.rs`, GVN | **G/M** all four oracles branch-local for cold/siblings; Clang loop-preheader; cold VM 0.983× base, hot-loop 0.758× kill; gzip VM 0.943× base and text -2,000 B; all four backends checked | 982 unit + 377 regression + codec veto | unconditional entry hoist |

### 5.4 Frontend / IR — FE-01 … FE-22

| # | ID | P | Item | Files | Evidence | Accept | Do not |
|---|----|---|------|-------|----------|--------|--------|
| 93 | FE-01 | DONE | Core assignment/call/return constraints diagnosed in sema; legacy unspecified prototypes and anonymous SIMD identities handled conservatively | `sema/analysis.rs` | **M** `check_sema_constraints.sh`, full suites | diagnostics in sema | invent incompatible anonymous identities |
| 94 | FE-02 | P1 | TBAA / `restrict` → IR | IR attrs | **C** noalias grep empty | attribute on IR | pretend alias is TBAA |
| 95 | FE-03 | P2 | ExprId not pointer | lowering | **C** TODOs | stable ids | |
| 96 | FE-04 | P2 | Vector subscript | frontend | **C** | | |
| 97 | FE-05 | P2 | typeof diagnostic | | **C** | | |
| 98 | FE-06 | P2 | `__builtin_nan` payload | | **C** | | |
| 99 | FE-07 | P2 | Parser panic-mode | ideas | | | |
| 100 | FE-08 | P2 | `_Complex long double` | | **C** | | |
| 101 | FE-09 | P2 | Unsigned subint IR | `ideas/fix_ir_unsigned_subint*` | **C** | | |
| 102 | FE-10 | P1 | Alignment/range on IR for RA | IR | **C** | better homes | |
| 103 | FE-11 | P2 | `fastcall` fptrs | | **C** | | |
| 104 | FE-12 | P2 | Nested sizeof | | **C** | | |
| 105 | FE-13 | P3 | Use-def shared context | ideas high_use_def | **S** backlog | O(1) use | rewrite every pass at once |
| 106 | FE-14 | P2 | Builtin table unify sema/type_checker | | **C** | | |
| 107 | FE-15 | P2 | C11 UCN lexer | | **C** | | |
| 108 | FE-16 | P1 | Volatile through peephole | prologue `LCCC_VOLATILE_SLOT` | **C** | volatile preserved | fold volatile |
| 109 | FE-17 | P2 | va_arg_pack stub | | **C** | | |
| 110 | FE-18 | P2 | Dependency file `-M` only source | pipeline TODO | **C** | | |
| 111 | FE-19 | P2 | **`__builtin_cpu_supports` runtime CPUID** (was P0 — now FIXED: exact Raptor Lake allowlist at `expr_builtins.rs:453` `PRESENT`; downgrade to P2 for runtime CPUID) | `expr_builtins.rs:436` | **C** FIXED: exact allowlist, not "return 1" | runtime CPUID on non-v3 targets | re-enable the "return 1 for everything" SIGILL bug |
| 112 | FE-20 | P2 | **`usual_arithmetic_conversion` edge cases** (was P0 — now FIXED: correct C11 6.3.1.8 else-arm at `types.rs:1586`; downgrade to P2 for edge-case tests) | `common/types.rs:1586` | **C** FIXED: `size > ? signed : unsigned_version` | edge-case regression tests | re-enable the `1LL + 1UL` wrong-type bug |
| 113 | FE-21 | P1 | **`_Pragma` implementation** (currently consumed and dropped) | `macro_defs.rs:542` | **C** C99 §6.10.9 violation | `_Pragma("foo")` injects `#pragma foo` | |
| 114 | FE-22 | DONE | C2x `__VA_OPT__` expands balanced replacement tokens before #/## handling; supports empty/nonempty, nested commas, token paste, and GNU named variadics | `macro_defs.rs` | **M** runtime regression covers five forms | selected tokens rescan normally | treat whitespace-only variadic as present |

### 5.5 ABI / aggregates / stack — AB-01 … AB-15

| # | ID | P | Item | Files | Evidence | Accept | Do not |
|---|----|---|------|-------|----------|--------|--------|
| 115 | AB-01 | P0 | SysV SSE-class return/args in XMM not rax shuttle | ABI + float_ops | **S** struct_copy 21× | XMM class | rax roundtrip |
| 116 | AB-02 | P0 | sret + one wide copy | ABI | same | one ymm/xmm copy | field loop |
| 117 | AB-03 | P1 | 4-byte slots without zext bug | stack_layout README | **C** | | |
| 118 | AB-04 | P1 | Overalign param copy already special-cased; audit | prologue Alignas(32) | **C** | | |
| 119 | AB-05 | P1 | Parallel copy param prestores remaining | prologue gpr_prestores | **C** | fewer cycles | |
| 120 | AB-06 | P2 | RISC-V va_arg long double struct | current_tasks | **C** | | |
| 121 | AB-07 | P2 | i686 double param high word | current_tasks | **C** | | |
| 122 | AB-08 | P2 | ARM KVM VA layout | ideas | **C** | | |
| 123 | AB-09 | DONE | Call-free x86 leaves whose layout used only the conservative callee-save collision reserve now elide the duplicate local frame; push/pop and CFA remain correct | x86 prologue | **M** removes `subq/addq $24` from two-exit leaf; targeted runtime/CFI regression; gzip 30/30 | `CCC_NO_EMPTY_LOCAL_FRAME_ELISION` | elide when calls/slots/varargs exist |
| 124 | AB-10 | P2 | Postgres frame size | ideas | **C** | | |
| 125 | AB-11 | P1 | Alloca escape analysis more GEP | stack_layout | **C** | | |
| 126 | AB-12 | P2 | Two coalescers unify | RA vs stack | **C** | one policy | |
| 127 | AB-13 | P2 | i686 boot 32K | external_tools TODO | **C** | | |
| 128 | AB-14 | DONE | Preserve positional parameter-Alloca prefixes and fuse adjacent F128 Load→Store into exact 16-byte backend memcpy; fixes pointer-as-x87-argument corruption and removes approximation/intermediate churn | DCE + shared generation | **G/M** runtime exact; LCCC 15→8 instructions (oracles 3); unit prefix invariant; cross-backend 16-byte copy path | correct stack ABI/runtime | delete earlier live-prefix homes |
| 129 | AB-15 | P1 | Dead XMM save in integer varargs remaining | vararg_fp_save | **C** | | |

### 5.6 PGO / IPO / layout — PG-01 … PG-12

| # | ID | P | Item | Files | Evidence | Accept | Do not |
|---|----|---|------|-------|----------|--------|--------|
| 130 | PG-01 | P2 | Sample PGO | ideas high_sample | **C** | | |
| 131 | PG-02 | P2 | Loop-versioned devirt | ideas | **C** | | |
| 132 | PG-03 | DONE | Exact pre-pass profile trip counts feed active per-loop vector profitability | OP-04 | **C/M** threshold units + active call site | short/large loops vetoed | ignore profile |
| 133 | PG-04 | P1 | RA use block counts after RA-01 | pgo_weight | **C** | gzip gate | `PGO_WEIGHT_MAX>1` first |
| 134 | PG-05 | P2 | Value specialization | ideas | **C** | | |
| 135 | PG-06 | P1 | Keep conservative layout | **M** expat | no 131→248 | reorder hot loop |
| 136 | PG-07 | P2 | LTO | none | | | |
| 137 | PG-08 | P1 | Force-inline budget vs frame | **C** 8B/SSA | | sqlite spike |
| 138 | PG-09 | P2 | Switch profile density vs bt | switch_dispatch 1.40× | **S** | | |
| 139 | PG-10 | P2 | Function placement | | | | |
| 140 | PG-11 | P1 | Flat-profile gate keep | integrated PGO | **M** adler −17% pre-fix | keep `has_spread` | drop gate |
| 141 | PG-12 | P1 | Don't promote 95% single-target | `LCCC_PGO_PROMOTE_STABLE=95` | **M** +28% if promoted | keep | promote stable |

### 5.7 Linker / assembler / multi-arch — LK-01 … LK-18

| # | ID | P | Item | Files | Evidence | Accept | Do not |
|---|----|---|------|-------|----------|--------|--------|
| 142 | LK-01 | P2 | mmap I/O | ideas | | | |
| 143 | LK-02 | P2 | `--icf` | linker | | | |
| 144 | LK-03 | P2 | RISC-V PIC auipc | **C** | | | |
| 145 | LK-04 | P2 | ARM `caspal`, `.org`, PREL64, movw | current_tasks | **C** | | |
| 146 | LK-05 | P2 | RISC-V dash FAIL | current_tasks | **C** | | |
| 147 | LK-06 | P2 | RISC-V kernel boot hang | ideas | **C** | | |
| 148 | LK-07 | P2 | `-mno-sse` full | mod.rs TODO | **C** | | |
| 149 | LK-08 | P2 | Encoder completeness | `encoder/mod.rs` TODO | **C** | | |
| 150 | LK-09 | P2 | i686 archive merge | TODO | **C** | | |
| 151 | LK-10 | P2 | R_X86_64_32 PIC | TODO | **C** | | |
| 152 | LK-11 | P2 | RISC-V `@cc` | TODO | **C** | | |
| 153 | LK-12 | P2 | RVV masked | TODO | **C** | | |
| 154 | LK-13 | P2 | Multi-alt asm ARM/RISCV | TODO | **C** | | |
| 155 | LK-14 | P1 | `encdiff.py` / `insndiff.py` in RA ISel loop | scripts | **C** | | |
| 156 | LK-15 | P2 | ELF32 script setup.elf | ideas | **C** | | |
| 157 | LK-16 | P2 | `--emit-relocs` | | | | |
| 158 | LK-17 | P2 | pcre2 792-nest frame | current_tasks | **C** | | |
| 159 | LK-18 | P3 | QEMU build | ideas | **C** | | |

### 5.8 Measurement / process / compile-time — MS-01 … MS-09

| # | ID | P | Item | Files | Evidence | Accept | Do not |
|---|----|---|------|-------|----------|--------|--------|
| 160 | MS-01 | P0 | Godbolt compare in CI for 6 kernels | `scripts/godbolt.py` | procedure §1 | CI artifact | skip oracles |
| 161 | MS-02 | DONE | `CCC_TRACE_ALLOCSTATS[=filter]` emits one deterministic aggregate line per selected function: eligible/scan/assigned/spilled/segments/holes/callee/caller homes | RA | **M** structural regression verifies fields, filtering, and byte-identical assembly | number in logs | alter allocation while tracing |
| 162 | MS-03 | P1 | CI correctness+regression+fuzz smoke | ideas README | **C** | | |
| 163 | MS-04 | P1 | OnceLock env knobs break parameterized tests | live_range.rs | **C** | tests independent | |
| 164 | MS-05 | P2 | Compile-time: pass rescans | use-def FE-13 | **C** | | |
| 165 | MS-06 | P1 | Document PhysReg map once | RA-19 | **C** | | |
| 166 | MS-07 | DONE | Current PR162+treatment: gzip 1.14 30/30, `longest_match` 330 instructions/118 stack refs vs pristine 331/119; CRC scale-4 SIB retained | workloads | **M/G** independent base build; gzip VM 0.943×, text -2,000 B | numbers on this SHA | claim VM result as hardware |
| 167 | MS-08 | P1 | **Consolidate 29+ `CCC_` env vars into `RaConfig` struct** | `regalloc.rs` (29), `live_range.rs` (7), `prologue.rs` (16) | **C** 29 in regalloc.rs alone, mixed polarity | `RaConfig` struct, single source of truth | env vars in hot path |
| 168 | MS-09 | P1 | **Audit `peephole_common.rs` UTF-8 fix impact** — old `bytes[i] as char` corrupted non-ASCII bytes in asm (FIXED in current main); verify no compiled binary shipped with corrupted asm | `engineering/evidence/` | **C** old code corrupted UTF-8 | audit report | re-enable `bytes[i] as char` |

---

## 6. File map (line numbers verified at `35a6b88`)

```
src/backend/regalloc.rs               3687  RA policy (29 CCC_ env vars)
src/backend/live_range.rs             1261  scan (OnceLock env cache, register_steal_is_safe, reg_hint field at :73)
src/backend/liveness.rs               2163  worklist dataflow, segments, gep_base_values
src/backend/split_ranges.rs           1377  IR split (now correct: rewrites uses, phi-safe)
src/backend/x86/codegen/machinst_regalloc.rs  635  DEAD CODE — DELETE (P0-01a, RAX-clobber bug)
src/backend/stack_layout/graph_coloring.rs  131  KEEP — wire after RA-23 (P0-01b)
src/backend/stack_layout/copy_coalescing.rs 2246  immediately_consumed blocker (RA-23)
src/backend/generation.rs             3622  ISel driver
src/backend/x86/codegen/prologue.rs   1815  RA call, MachInst gate, folds
src/backend/state.rs                        explicit SlotAddr::Reg(PhysReg) (RA-24 DONE)
src/backend/peephole_common.rs         408  UTF-8 safe, tab-aware (FIXED)
src/backend/x86/codegen/memory.rs     memcpy ymm
src/backend/x86/codegen/float_ops.rs  FMA scalar
src/backend/x86/codegen/alu.rs        lzcnt/tzcnt/popcnt emitters
src/backend/x86/codegen/machinst*.rs  dual codegen (RA module dead — ISel/emit path alive)
src/passes/loop_memory_promote.rs     1712  object_root, memory_on_exit (FIXED); duplicated alias engine (RA-25)
src/passes/alias.rs                    128  LoopFrames, resolve_in_frame, forms_disjoint (underutilized)
src/passes/licm.rs:750                 TODO GEP alias (P0-04)
src/passes/vectorize.rs               6905
src/passes/inline.rs                  3719
src/passes/aggregate_sroa.rs          1009
src/passes/outline_switch.rs:37       MIN_CASES = 40 (FIXED)
src/pgo/unroll_pgo.rs                 active per-loop PGO vector cost (OP-04 DONE)
src/frontend/sema/analysis.rs         core assignment/call/return constraints (P0-05 DONE)
src/ir/lowering/expr_builtins.rs:436  __builtin_cpu_supports (FIXED: exact allowlist at :453)
src/common/types.rs:1586              usual_arithmetic_conversion (FIXED: C11 else-arm)
scripts/godbolt.py                    CE oracle
engineering/evidence/workloads/
```

## 7. What "beat ICX" means

- **Integer codecs:** remat+RA+SIB (match, Adler, CRC) then ICX XMM-spill if still behind.
- **FP:** copy **ICX** `vfmadd231pd` YMM accumulator, **not** GCC 16.2 per-iter horizontal add.
- **Aggregates:** SysV register class + ymm copies (21× is the largest **S** gap).
- **Parsers:** `btq`/range, segment RA, do **not** mega-inline (varint experiment failed).
- **Bit kernels:** gcc `andn`+`cmov` on find_bit; `popcntl` on bitops; **do not** copy clang sieve ymm.

## 8. What landed since `3e0ab3e` (do not re-fix)

- ✅ **`live_range.rs` Grok variant** — OnceLock env cache, `register_steal_is_safe`, binary search, `run()` idempotency.
- ✅ **`liveness.rs` Grok variant** — worklist dataflow (no `MAX_ITERATIONS` cap), two-latch loop-depth fix, F128 resync, point-level setjmp, GPR-clobber asm.
- ✅ **`split_ranges.rs` Grok variant** — call-split now rewrites uses; loop-split scans all body blocks, phi-safe, span-preserving.
- ✅ **`peephole_common.rs` Grok variant** — UTF-8 safe, tab-aware, `LineStore::replace` no-leak, `replace_source_reg_att`.
- ✅ **`loop_memory_promote.rs` Grok variant** — `object_root` identity, `memory_on_exit` do-while fix, `inst_may_clobber_unknown`, speculation safety.
- ✅ **`outline_switch` MIN_CASES = 40** (was 999999).
- ✅ **`__builtin_cpu_supports` exact allowlist** (was "return 1 for everything"; `PRESENT` const at `expr_builtins.rs:453`).
- ✅ **`usual_arithmetic_conversion` C11 else-arm** (was wrong for `1LL + 1UL`; `types.rs:1586`).
- ✅ **`LivenessResult::segments`** — hole-aware intervals, consumed for call-spanning (scan still fat).
- ✅ **`alias.rs`** — `LoopFrames`, `resolve_in_frame`, `forms_disjoint` (consumed by `redundant_loads` only).
- ✅ **Engineering doc tree** — `engineering/{STATE.md, agent/{BACKLOG,RULES,SEQUENCE,README}.md, evidence/godbolt/}`.
