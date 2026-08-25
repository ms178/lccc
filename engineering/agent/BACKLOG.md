# 150 highest-ROI work items

Ranked work for **generated-code performance** vs GCC 16.2 / Clang 22.1 / ICC 2021.10 / ICX.
This file **is** the 150-item catalog (IDs P0 / RA / IS / OP / FE / AB / PG / LK / MS). Do not replace it with a stub.

Evidence: **M** measured LCCC vs GCC14 on kernels; **G** Compiler Explorer (`cg162` / `cclang2210` / `cicc2021100` / `cicxlatest`, `-O2 -march=x86-64-v3`); **C** tree fact (line numbers verified at `35a6b88`); **S** screening suite.

**ROI rank:** P0 this week (testable) · P1 this month · P2 after P0/P1 · P3 research.

Count: P0 5 + RA 27 + IS 28 + OP 34 + FE 22 + AB 15 + PG 12 + LK 18 + MS 9 = **170** (extra items are refinements; drop `[NEW]`/`[REFINED]` tags when stable).

### Peephole/RA ledger (kernel corpus + `tests/benchmark/programs` A/B + Godbolt oracle)

Harnesses: `scripts/kernel_count.py` (per-function counts vs system GCC),
`scripts/peephole_ab.py` (whole-corpus A/B **with a behaviour comparison**) and
`scripts/godbolt.py` (the scoreboard oracle).

Kernel corpus total: **375 → 325** against GCC's 264. Benchmark corpus A/B:
**7060 → 6705 instructions (−5.03 %)**, 38/38 programs behaviour-identical.
Compiler Explorer at `-O2`: LCCC is the best of {LCCC, GCC 16.2, Clang 22.1,
ICX} on **sum8 12** (GCC 57), **cntz 16** (GCC 68), **maxv 19** (GCC 27),
**dot 15** (GCC 17) and **hsh 14** (GCC 15), and ties on **copy64 9** and
**my_strlen 10**.

| ID | P | Item | Evidence | Status |
|----|---|------|----------|--------|
| PF-00 | DONE | FP binop wrote the RHS into the destination holding the LHS (`x + 1.5` computed `1.5 + 1.5`) | regression 462→475 | fixed (session 74) |
| PF-01..03 | DONE | move relay, windowed lea fold, setcc/test/cmov collapse | — | shipped |
| PF-04 | DONE | **dominance-correct liveness**: real CFG dataflow (`peephole/passes/liveness.rs`) is now the primary deadness answer; the two syntactic proofs remain as a union for sub-register cases the dataflow must be pessimistic about | unlocked PF-08..PF-14 | shipped |
| PF-05 | P0 | RA-03: adler keeps eight unrolled byte temporaries live and spills `s1`/`s2` | adler 89 vs GCC 66 (oracle) | open, largest gap |
| PF-06 | P1 | isort: `(j+1)*4` strength reduction and `int` index widening (`cltq`/`movslq` chains) | isort 43 vs Clang 23 | open |
| PF-07 | P1 | LICM for `leaq sym(%rip)` in loops. x86-64 cannot combine a rip-relative displacement with an index register, so the earlier "crc SIB fold" entry was wrong — hoisting is the fix | crc 28 vs GCC 17 | open |
| PF-08 | DONE | producer retargeting and store relay (`movl %A,%D; movl %D,MEM` → `movl %A,MEM`) | gzip_crc32 −26 | shipped |
| PF-09 | DONE | copy+add and copy+shl folded into `lea` (flag-neutral) | isort 47→43 | shipped |
| PF-10 | DONE | copy+mask → `movzx` | crc | shipped |
| PF-11 | DONE | **copy coalescing**: rename the allocator's home onto the ABI register and retire the parameter shuffle | sum8 14→12, bswp32 13→11, mix6 16→13 | shipped |
| PF-12 | DONE | dead pure-write elimination (LEA / `movz*` / `setCC` / `cmov` / copies) | bswp32, lz, copy64 | shipped |
| PF-13 | DONE | load + self-test → `cmp $0, mem` — needs liveness ACROSS the branch, which is why it was rejected in session 74 | strlen 12→10, ties GCC | shipped |
| PF-14 | DONE | accumulator round-trip applied in place, redundant self-test after a logical op, dead sign-extension narrowing, repeated-load reuse | hsh 18→14, ffs1 18→17, lz 9→8 | shipped |
| PF-15 | P1 | scmp re-sign-extends the same byte three times (`movsbq %dil, %rsi` after `movsbq (%rbx), %rdi`): "the source is already its own extension" | scmp 25 vs GCC 12 | designed |
| PF-16 | P2 | `movq $imm32, %reg` → `movl $imm32, %reg`; `cltq` after `lzcnt` is provably redundant | ffs1 17 vs 11, lz 8 vs 6 | open |
| PF-17 | P2 | loop rotation (bottom test): −1 taken branch per iteration but +1 static instruction — needs a cycle-level gate, the instruction-count proxy would call it a regression | every loop kernel | open |

Rules encoded as unit tests in `liveness.rs`, `copy_coalesce.rs`,
`dead_writes.rs`, `flag_peepholes.rs` and `relay_and_lea.rs`: `%r10` is the
static chain (family **10**, not 11) and is live into every call; `%cl` is a
fixed operand of the shift instructions and must never be renamed; `%rdx` is
live at `ret` unless the epilogue demonstrably returns one integer in `%rax`;
32-bit writes zero-extend, which makes the width rules asymmetric in both
directions; EFLAGS being block-local is verified per function, not assumed; and
a label with no incoming branch is a fall-through edge, not a join.

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
| Expat name scan | **1.69×** (v12) | FNV-prime IS register-homed (%r14); gap is loop rotation + byte-classify verbose vs GCC case-fold | gcc `btq`; gcc keeps FNV prime in `%r10` |
| xmltok.c TU | **12×** stack-mem | **RA segments** | — |
| inflate.c TU | **15×** stack-mem | **RA segments + switch** | — |
| `struct_copy` | **1.54×** (41 vs 26 ms; was 21.06×, then 3.11×) | aggregate copy-forward fixed (param-set staleness, GEP-hoist, window over-rejection); remainder = scalar movsd chains vs GCC's paired mulpd | gcc/icx: 2× ymm for 64 B `*dst=*src` |
| `dot` (int, loop_patterns) | **scalar** (1.99× with find_max) | **needs `vpmuldq` widening-mul intrinsic** + vectorizer dot for I32→I64 | gcc scalar too; widening-mul can beat |
| double I32 reduction | **0.7251×** stack-home control (95% CI 0.6007..0.8807); **0.895× GCC** | integer reduction homes complete; shared-load/temp forwarding remains | 215 vs control 227 instructions; 39 vs 63 stack refs |
| multi-use SSE sat chain | register homes **0.9410×** stack control (95% CI 0.9182..0.9627) | **RA-21 done**; remaining loop-invariant/DSE gap | LCCC 58 insn; GCC 19, Clang 11, ICC 16, ICX 23 |
| nbody / spectral / mandelbrot | **1.26 / 1.30 / 1.23** (v12; FP staging-copy elimination + cmp-branch fusion) | remaining: loop rotation + non-reduction FP YMM vectorize | ICX nbody 97 ymm / 58 FMA; gcc 0 ymm / 24 FMA |
| sqlite varint | **1.34×** (v11) | **ISel branches**, not inline | — |
| linux_find_bit | **1.42×** (v11) | gcc **`andn`+`cmov`** on C ffs tree, **not tzcnt** | CE corpus |
| loop_patterns | **0.98× (beats GCC)** (v12; LCG RA precise-span seed + fused mul-add + widening accumulator coalescing) | find_max AVX2 transform wired + correct but detection gated pending cost-model; int dot_product scalar (vpmuldq v13) | gcc `vpmaxsd` would beat its scalar |
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
| 26 | RA-21 | DONE | Fail-closed exact vector-argument prefixes plus width-aware x86 reduction homes now cover F32/F64, I32 AVX2/SSE2, and I64x2 sum/dot webs; integer zero/horizontal/multiply emitters honor assignments; an XMM2 scratch quarantine may start the x86 pool at XMM3 without disabling it | `regalloc.rs`, `generation.rs`, x86 intrinsics + regalloc helpers | **M/C/G** SSE sat 0.9410× stack control; double-I32 reduction 215 vs 227 instructions, 39 vs 63 stack refs, paired VM 0.7251× (95% CI 0.6007..0.8807), 23/24 wins; current LCCC 0.895× GCC | AVX2/SSE2/I64 runtime + structural kill-switch + PMOVZX regression | classify scalar/immediate args as vector pointers or open generic SIMD wholesale |
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
| 33 | IS-02 | DONE for returns (2026-08-23 session 64): all-SSE 16-byte structs return xmm0/xmm1 (SetReturnF64Second); inline candidates keep I128; field-store/arg halves remain (IS-02b) | `stmt_return.rs`, `returns.rs` | **M** mk 24→11 insns; cross-ABI interop both ways | mov count ≤40 | keep rax shuttle |
| 34 | IS-03 | DONE | AVX2 64-byte assignment → 2 YMM load/store pairs + `vzeroupper`; safe leaf DCE removes dead parameter homes and the entire frame/setup | x86 memcpy + target feature contract + DCE | **G/M** 6 instructions, exact parity with GCC16.2/Clang22.1/ICX latest (ICC 9); runtime exact; baseline ISA remains XMM | structural/runtime regression | use YMM for 32/48 B (measured slower) |
| 35 | IS-04 | DONE | Contract-legal dot reductions emit `vfmadd231ps/pd` into an assigned YMM accumulator and perform one horizontal reduction after the vector loop | vectorizer + x86 reduction emitter | **M/G** `p17/p18` direct YMM2 FMA; `check_vector_dot_fma_codegen.sh` + reduction-home gate | FMA in loop, one exit horizontal | copy gcc16 per-iter hadd |
| 36 | IS-05 | PARTIAL | Reduction FMA folds one proven source stream as a memory operand; strict non-contracted `vmulps/pd` still round-trips the other `VecLoad` through a temporary slot | x86 vector emitter / vector-load forwarding | **M/G** FMA shape complete; strict p17/p18 retain one store + stack memory operand | direct stream memory operand without alias/liveness regression | fold aliased |
| 37 | IS-06 | DONE (2026-08-21 session 38) | Cross-target BitTest canonicalization | `ir/ops.rs`, `simplify.rs`, all backends | **G/M** `(x >> i) & 1` becomes `BitTest`; x86 uses BT, AArch64 UBFX for constant indexes, RISC-V/i686 use shift/AND | `check_bit_test_canonical.sh` |
| 38 | IS-07 | P1 | `cmov` limit idiom | `if_convert.rs` | **G** gcc match `cmovc` | cmov in match | if-convert huge blocks |
| 39 | IS-08 | P1 | Load-cast fold remaining | `prologue.rs` load_cast | **C** 33% gzip-9 was movzbl | leftover movzbl down | disable `CCC_NO_LOAD_CAST_FOLD` globally without measure |
| 40 | IS-09 | P1 | Cmp-replay only from **slots** | keep | **C** sqlite `yy_shift` | no sqlite SEGV | replay from physreg |
| 41 | IS-10 | P1 | Flag fusion Copy/Cast chain | exists; extend carefully | **C** expat accounting SEGV if multi-use | more fused cmp | multi-use flags |
| 42 | IS-11 | P1 | C `__ffs` if-tree → gcc **`andn`+`cmov` chain** (not tzcnt) | `bit_idioms.rs`, `alu.rs` | **G** gcc cmov×11, **not** tzcnt; 1.85× | find_bit CE vs gcc | blindly emit tzcnt |
| 43 | IS-12 | DONE | Adjacent single-use `not`+`and` → BMI1 `andn{l,q}`, direct physical sources, baseline ISA gated | CodegenOptions + shared fusion + x86 ALU | **M** linux_find_bit 1.04x faster vs control, 1.42x behind GCC | BMI/no-BMI runtime+assembly regression | emit without BMI contract |
| 44 | IS-13 | P1 | ALU+mem for spilled | peephole | ICX lookalike | fewer reloads | change RA homes |
| 45 | IS-14 | DONE (2026-08-21 session 37) | 32-bit integer returns now use `operand_to_rax_i32`; self-moves remain covered by the existing `SelfMove` peephole | `returns.rs`, `emit.rs`, `check_return_32bit_zeroext.sh` | **C/M** leaf boolean/classifier returns use `movl`/`xorl`, not `movq` | structural regression | delete live copies |
| 46 | IS-15 | PARTIAL→DONE for SIB loads (2026-08-23 session 64) | Indexed I32 loads use `mov_load_for_value` (sext-analysis aware): `movl` when no 64-bit consumer | memory.rs emit_load_indexed_common | **M** structural gate; window[i] now movl | fewer movslq | break GEP signedness |
| 47 | IS-16 | P2 | Match inner 258 B with `pcmpeqb` | vectorize | spike | gzip gate | ship without CRC/adler oracles |
| 48 | IS-17 | P1 | MachInst profit model ≠ 32-inst cutoff | `prologue.rs:812` | **C** gzip −3% | beat gzip then raise | `CCC_MI_FORCE_LOOPS` |
| 49 | IS-18 | P2 | MachInst fused cmp/select | declined | **C** | — | force on gzip |
| 50 | IS-19 | P2 | Text peephole vs MachInst conflict | `CCC_NO_MACHINST` | **C** | documented interaction | dual-apply same fold |
| 51 | IS-20 | DONE (2026-08-23 session 64): `dirty_upper_ymm` flag set by 256/512-bit intrinsic ops + YMM load/store paths, emitted in epilogue; explicit-cleanup sites disarm | state.rs, intrinsics*, prologue | **M** intrinsic copy64 emits vzeroupper before ret | AVX transition safe | always vzeroupper |
| 52 | IS-21 | P2 | Scheduler already lost gzip 3% | emit / prologue | **C** `-mtune` diagnostic only | PMU after ISel | turn on as default |
| 53 | IS-22 | P1 | Indexed GEP SIB completeness | generation + memory | **S** loop_patterns 1.85× | SIB in loops | GOT for file-scope |
| 54 | IS-23 | P2 | `repe cmps`/`pcmpeqb` memcmp | | noisy <20 ms | scale work first | optimize 20 ms kernel |
| 55 | IS-24 | DONE (2026-08-23 session 64) | Immediate stores fire through register-homed (`SlotAddr::Reg`) and over-aligned bases in both `emit_store_impl` and the const-offset path; no `movq $imm,%rax` round-trip remains | memory | **M** structural gate `check_imm_store_movl_sib.sh` | fewer rax materializations | regress SpilledALU paths |
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
| 65 | OP-06 | DONE | F32/F64 and I32/I64 reduction webs keep packed accumulators through the vector loop and horizontal-reduce once at vector exit before the scalar remainder | vectorizer + width-aware x86 homes | **M/G** p15–p18 and integer reduction structural gates; no packed horizontal in loop | one horizontal sequence at exit | gcc16 per-iteration pattern |
| 66 | OP-07 | P1 | Runtime alias versioning | none | **C** | versioned loops, checksums | version everything |
| 67 | OP-08 | P1 | Nested IVSR with dependence | `iv_strength_reduce.rs` skip overlap | **C** nested-matmul wrong if forced | loop_patterns + matmul | force overlapping |
| 68 | OP-09 | P2 | Enable `univsr` after SSA repair | gated | **C** ideas | | enable gated pass raw |
| 69 | OP-10 | P1 | Unroll vs RA pressure | `loop_unroll.rs` | **C** | gzip unregressed | unroll longest_match blindly |
| 70 | OP-11 | P2 | Loop interchange matmul jk | vectorize / loop | **S** | matmul vs ICX | interchange dependent |
| 71 | OP-12 | DONE for store-forwarding keys (2026-08-23 session 64): canonical address VNs unify site-local GlobalAddr GEPs; FP stores forwardable; Copy VN-transparent | `gvn.rs` | **M** g1[i] forwards to constant (GCC parity); may-alias counter-case pinned | fewer redundant loads | CSE aliased |
| 72 | OP-13 | DONE for same-block stores (2026-08-23 session 64): new `dse.rs` backward scan, closed-alloca escape analysis, byte-range kills, `CCC_NO_DSE`; cross-block remains follow-up | `dse.rs` wired into -O1 + main loop | **M** overwritten stores eliminated (GCC parity on probe); unit+regression tests | dead stores gone; alias guard pinned | DCE volatile |
| 73 | OP-14 | P1 | If-convert >8 inst / more diamonds | `if_convert.rs` | **C** | more cmov | convert huge blocks |
| 74 | OP-15 | DONE | `set_membership.rs` v2: skip-tolerant clustering (`\|\|`-commutativity over pure tests), ASCII case-fold merge (`andl $-33`), best-window selection, head-prelude tolerance; BitTest→CondBranch fusion (`bt; jc`, zero materialization, both x86 backends, `CCC_NO_BT_BRANCH_FUSION`); fixed 4 latent BitTest/redundant_ext bugs (2 miscompiles fuzz-found, 1 -m32 ICE, 1 i686 stale-%ecx BT) | set_membership + generation + traits + both comparison.rs + alu.rs + redundant_ext + simplify | **M** xml_name_continue 45→28 insns (gcc 20); exhaustive 0..255 parity -m64/-m32; IR-interpreter unit tests; fuzz 500/500 | btq classify | single-block And materialization (measured +5 insns) |
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
| 91 | OP-32 | PARTIAL (2026-08-23 session 64): symbol-based GEP keys unified via canonical address VNs; restrict-param `forms_disjoint` wiring remains | `gvn.rs`, `alias.rs` | **M** cross-site store forwarding now fires | fewer redundant loads | CSE aliased |
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
| 113 | FE-21 | DONE (2026-08-23 session 64): de-stringizing operator queued during expansion, drained per-line/EOF into `handle_pragma`; push/pop_macro round-trip matches GCC | `macro_defs.rs`, `pipeline.rs` | **M** `pragma_operator.c` vs gcc identical | `_Pragma("foo")` injects `#pragma foo` | |
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
| 127 | AB-13 | DONE (2026-08-25 session 82) | i686 Linux setup 32 KiB gate | natural 4-byte i686 slots + `.loc`-transparent peepholes + measured `-m16 -Os` phase policy + ELF32 script `--gc-sections`/`KEEP` | **C/M** patched linux-cachymod 6.18.44: `_end=0x79c0`, 1,600 B headroom; 26,208-byte flat image byte-identical across lccc-ld/BFD/LLD; `.pecompat` and `.videocards` retained | authentic ASSERTs + ELF32 GC oracle + m32/regparm fuzz | weaken ASSERT or drop protocol registries |
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
- ✅ **Session 44 (2026-08-22)** — re-based onto PR #179; `hot_loop_home` Phase-1 candidacy now keys on the hottest **use-site** loop depth (preheader-defined IVs get callee-saved homes; ra01 proxy 124→117 insns / 34→20 stack refs); ABI-hinted ParamRefs are never evicted (`select_evict_victim`); rbp(6) accepted as a callee-saved prestore home; the fallback ParamRef ABI read is pinned via `# LCCC_PARAM_ABI_READ`; `vector_temp_promotion` constant-address load sources survive only alloca-confined writes; `gen_lcccsimd.py` imm-builtin protos no longer declare a trailing `int __imm` (t128/t256/t512 intrinsics compile again); `tests/regression/run_regression.py` (the missing-in-repo corpus runner) plus `.env` oracle opt-outs for three host/feature-dependent tests. Zero-future-use eviction search measured (Adler +15%) and **reverted** — do not resurrect without the Adler/gzip oracles. See `docs/history/2026-08-22-session44-rebase-soundness-sweep.md`.
- ✅ **Session 47 (2026-08-22)** — `eliminate_never_read_stores` revived for no-FP functions with callee-saved push chains.  Two defects made the form-2 (`subq $N,%rsp`) prologue detector unreachable: the form-1 failure path (`pushq %rbp` NOT followed by `movq %rsp,%rbp` — the normal no-FP case, since lccc allocates rbp as a callee-saved register) skipped one line PAST the subq, and form-2's fixed 3-line window missed the .cfi directive behind six pushes.  Net effect: dead-slot-store elimination silently skipped every such function (gzip CRC kernel kept a never-read `movq %rax, 8(%rsp)` in its inner loop).  Fix: fall through on form-1 mismatch; scan back up to 16 lines past push/nop lines, fail closed on any other terminator.  CRC TU: main 118→112 instructions. New unit test pins the six-push no-FP shape.  Follow-up root causes documented in `docs/history/2026-08-22-session47-nrs-prologue-detection.md`: RA-03 DO8 needs RA-06 splitting (eviction-mode A/B all worse), CRC PIE base-remat needs an OP-34 preheader-home relaxation, sema const-assignment needs per-pointer-level qualifiers (FE-02), and the `movl %eax,%eax` artifact needs load-width-directed folds (IS-08) — not deletion.  Validated: 1075 unit, 382/389 regression, intrinsics 3/3, gzip 30/30 + roundtrip, zlib-ng ctest=8 roundtrip=0, expat 2/2, phi 1800/1800, differential 1500/1500, alias-m32 1080/1080.
- ✅ **Session 46 (2026-08-22)** — MachInst partial-register segfault class closed. `lower_cast` (x86-64 MachInst ISel) lowered every narrowing/same-size cast as a truncating `movb`/`movw`/`movl`, leaving the destination register's upper bits stale; the folded-SIB index path then read a never-materialized cast result from its (die-at-birth-shared) home at 64-bit width — zlib-ng `zng_emit_dist`'s `extra_dbits(%rcx,%r10,4)` segfaulted OOB. Narrowing/same-size casts now emit extending moves (`movzbl`/`movsbl`/`movzwl`/`movswl`/`movslq`; `movl`-via-`Movzx` so a die-at-birth self-move can never skip the extension; `movl`/`movq $imm` for constants), matching the mature path and GCC/Clang.  The IR always widens sub-32-bit values through an explicit Cast before wider uses, so extending every cast home closes the whole class.  New regressions: `session46_dist_code_index.c` (the zlib shape) and `session46_narrow_cast_torture.c` (U8/U16/I8/I16 × index/shift/mul/select/call/chain, GCC-differential).  Validated: 1046 unit (3 new `lower_cast` tests), 375/383 regression, intrinsics 3/3, zlib-ng 2.3.3 gate ctest=8 roundtrip=0, gzip 1.14 30/30 + roundtrip, expat 2.7.1 2/2 ctest, phi 1800/1800, differential 1500/1500, alias-m32 1080/1080.  See `docs/history/2026-08-22-session46-machinst-partial-register-segfault.md`.
- ✅ **Session 45 (2026-08-22)** — final red-team audit of vector-temp-promotion v3 (Agent B's patch, independently re-red-teamed): `Load256`/`Store256` keep their 32-byte alignment contracts (not forwardable/relaxable); promotion additionally requires `intrinsic_overwrites_full_result` (FMA/BroadcastLoadF64/store forms never rewrite their dest); `intrinsic_dest_reads_old_value` centralizes RMW destination semantics and feeds `single_read_sites` so write-only dests stop keeping forwarded loads alive (gzip PCLMUL −5 static insns); `pointer_roots` → `pointer_root_analysis` with iterative Kosaraju SCC + single-seed cycle shortcut (`p = phi(seed, p+stride)` now resolves, multi-seed cycles fail closed) + recurrence tracking gating the multi-use Loaddqu exception; wide scalar accesses retain natural alignment (I128/F128 on non-x86); pass order is promote → fuse → downgrade; O(1) load-result membership. Adoption audit fixed: module-wide `max_value_id` array sizing → dense per-function value-id renumbering, the quadratic terminator-loop lookup B missed, `cyclic_seed` simplification, and the complexity comment (O(V + Σ arity²), not flat O(V+E)). Kept the session-44 constant-address rule (alloca-confined writes provably cannot hit a fixed address). New stress regression `session45_vector_recurrence_stress.c`. See `docs/history/2026-08-22-session45-vector-promotion-v3-audit.md`.
- ✅ **Session 47** — multi-reduction vectorization: loops with TWO independent accumulators (`vBv += u[i]*v[i]; vv += v[i]*v[i]`, double running sums/dots) now vectorize on AVX2 + SSE2 via `ReductionPattern.second`/`analyze_secondary_accumulator`/`rewrite_reduction_body` (descending add-index order, one union soundness check, extracted `rewrite_accumulator_uses_outside_loop`). Dependent (`sum2 += sum1`), widening, and >2-accumulator shapes fail closed; single-accumulator path A/B-verified bit-identical. Evidence: new `double_reduction` benchmark 3.75× → **1.085×** vs GCC (3.5× over LCCC scalar); 1045 lib + 391 regression + 50 correctness + 360 fuzz. See `docs/history/2026-08-22-session47-multi-reduction.md`.
- ✅ **Session 50** — GVN per-object epochs for disjoint non-escaping allocas and `restrict` parameters (OP-32): `find_nonescaping_allocas` (owner-set fixpoint + escape scan) and `find_noalias_params` feed a `PtrBase` classification propagated single-source through GEP/ptr-Cast/Copy/Add/Sub; stores through classified objects bump only that object's epoch, `Unknown` still bumps the global generation. Eliminates the redundant `y[i]` reload across a disjoint `x[i]` store (`check_gvn_disjoint_epochs.sh` pins the restrict/plain/aliased cases). Battery hardening: gzip runner runs `make check` serially with one retry (the earlier "flakes" were a full /tmp tmpfs). **Found a pre-existing miscompile**: loop IV carried in caller-saved `%edi` across a call (`argv[++i]` shape) → zlib-ng `CVE-2018-25032`/`GH-382` CTest failures (62/69; roundtrip byte-identical; GCC build 100%). Reproducer committed at `tests/gauntlet-repros/loop_iv_across_call.c`. See `docs/history/2026-08-22-session50-gvn-disjoint-epochs.md`.
- ✅ **Session 49** — re-based onto PR #190 (which superseded the integer-home half of session 48 with I64x2 + XMM2-scratch quarantine) and kept the two still-unique pieces: `deduplicate_vector_loads` (one VecLoad per shared array per iteration, single- and multi-accumulator paths) and the N-accumulator lift (`seconds: Vec<SecondaryAccumulator>`, arbitrary independent zero-init chains, per-accumulator remainder chains). Conflicts resolved by taking upstream's regalloc/intrinsics verbatim and re-applying the vectorize deltas on top (incl. the new NEON `Max` reduction). See `docs/history/2026-08-22-session49-rebase-dedup-nacc.md`.
- ✅ **Session 48** — completed the highest-ROI session-47 follow-up: x86 reduction homes now cover I32x8/I32x4 and I64x2 sum/dot copy webs; integer zero/horizontal/software-multiply emitters honor assignments; the XMM2 scratch quarantine no longer disables a safe pool starting at XMM3. `double_reduction`: 215 vs 227 control instructions, 39 vs 63 frame refs, paired treatment/control **0.7251×** (95% CI 0.6007..0.8807), and current LCCC **0.895× GCC**. Added AVX2/SSE2/I64 structural control gate, generalized the paired benchmark script, made the i686 FMA regression libc-independent, and refreshed the stale Tier-2 frame assertion. Validated: 1075 unit, 381/390 differential (7 oracle + 2 host skips), 400/400 shell/structural, intrinsics 3/3, gzip 30/30 treatment/control. See `docs/history/2026-08-22-session48-integer-reduction-homes.md`.
- ✅ **Session 43** — red-team of Agent D's vector-temp-promotion v2: adopted (movnt align-16 relaxation via `intrinsic_dest_required_alignment`, unified width-typed `pointer_vector_arg_width`, non-escaping-alloca alias refinement, load-load chaining, single-read-site gating) and fixed two miscompiles it re-introduced — (1) `invalidate_for_value_write` dropped the slot-vs-write check (reassignment `v=load(p); v=setzero(); use(v)` read stale `p`), now a `write_may_clobber` slot+source test; (2) `VecStoreI64x2` writes memory through `args[1]` with `dest_ptr:None` but inherited the `produces_vector_value` register-result exemption. Also re-added the load-`dest` dead check and added the six 256-bit widening extensions (Pmovzx*/Pmovsx*256) to the forwardable/relaxable table. Measured: simd_movnt 422→390 insns / frame 440→392; simd_avx2_256 743→734 / 1016→824. See `docs/history/2026-08-21-session43-vector-promotion-v2-redteam.md`.
- ✅ **Session 42** — red-team audit of `vector_temp_promotion.rs` (late SIMD cleanup): adopted Agent D's alias-aware rewrite (escape-gated promotion window, object-root load fusion with write invalidation, fail-closed alignment whitelist, span-preserving removal) and fixed four audit-found defects — (1) slot-identity invalidation in `invalidate_for_pointer_write` (reassignment miscompile), (2) `Add`/`Sub` root propagation in `pointer_roots` (param-derived load sources were "may alias anything" and un-fused), (3) `LoadF64x4/2,LoadI32x8/4` downgrade whitelist narrowed to `index == 0` (offset arg), (4) fail-closed `dest_ptr: None` invalidation + load-`dest` dead check. A/B: 34-file corpus byte-identical; 1022 unit + 389 regression + 50 correctness + 60 fuzz green. See `docs/history/2026-08-21-session42-vector-temp-promotion-redteam.md`.
- ✅ **Session 41** — `redundant_ext.rs` SIB comma fix (zero-extend tracking re-enabled after indexed loads; also fixes a latent stale-byte-fact soundness bug on opaque SIB `movq` loads); 3-operand immediate `imul` for constant multiplies; BT bit-index consumed from its own register (no `%rcx` staging; `check_bit_test_canonical` oracle widened); 32-bit ALU writes tracked as upper-32-zero (enables `movq→movl` narrowing after `addl`/`subl` chains); `setup_oracles.sh` pins `MOLD_LTO=OFF` and records resolved oracle revisions. See `docs/history/2026-08-21-session41-x86-isel-peephole-sweep.md`.

## 9. glibc 2.44 campaign (sessions 54-55, 2026-08-22) — status + new items

**State:** configure passes with `CC=lccc` + `LD=lccc-ld` (`--disable-multi-arch
--disable-cet`, official tarball + `ms178-glibc.patch`). Entire compile phase
(every TU incl. malloc.c, all of elf/, nptl, libio, stdlib) compiles at
`-O2 -march=x86-64-v3 -fPIC`. rtld-libc.a machinery (relocatable links, map
scraping) works end-to-end. Fixes landed this campaign (do not re-fix):
`-print-prog-name=`, post-phi `Copy(Const)` Value-position soundness,
`__seg_fs`/`__seg_gs` declarator/typedef/local threading, `__CET__` GCC
parity, gnu89 extern-inline must-inline class, `-r` whole-archive/`.os`/`.oS`
inputs + `-Map` emission, sema vector_size typedef layout (shared cache
poisoning), return-void GNU compat (sema + lowering), scoped shadowing of
file-scope prototypes, SQLite-era four miscompiles (see session docs).

| # | ID | Pri | Item | Evidence | Notes |
|---|----|-----|------|----------|-------|
| 171 | LK-19 | **P0** | IFUNC end-to-end: `__attribute__((ifunc))` in sema/lowering, `.type %gnu_indirect_function` data refs, `R_X86_64_IRELATIVE` for data words in exec/shared links, ld.so-side semantics | **M** glibc configure rejects `--enable-multi-arch` (probe: `.quad ifunc-sym` produced no IRELATIVE; attribute produced no IFUNC symbol) | Gate for the ms178 PKGBUILD's multi-arch build; string-fn dispatch performance depends on it |
| 172 | LK-20 | P1 | `-r` output emits a spurious blank `UND NOTYPE LOCAL` symtab entry (index 3) | **M** `readelf -sW` on any `-r` product | Cosmetic-ish but GNU nm prints a bare `U `; fix in emit_rel symbol table build |
| 173 | LK-21 | P1 | `-l` library resolution in the driver's `-r` path (`-lgcc` in glibc's reloc-link is currently dropped) | **C** common.rs `-r` branch collects only bare paths | Soft-float/i128 helpers would go missing; x86-64-v3 got lucky |
| 174 | LK-22 | P2 | `-Map` for exec/shared links reaching the mapfile.rs writer from the -r path spellings | **C** | mapfile.rs exists; -r path writes a minimal member list only |
| 175 | FE-23 | P1 | Address-space qualifiers on function PARAMETERS (`void f(u64 __seg_fs *p)`) and multi-level positions (`T * __seg_fs *`) | **C** apply_declaration_address_space handles decl-level only | glibc THREAD_* macros mostly cast, so not blocking |
| 176 | FE-24 | P2 | `#error` message must be emitted verbatim (lccc macro-expands the token list: "96 must be multiple of 64" instead of "REGISTER_SAVE_AREA must be multiple of VEC_SIZE") | **M** dl-trampoline.h diagnostics | Diagnostics-quality only |
| 177 | MS-10 | P1 | `CCC_VALIDATE_SSA` in CI for the regression corpus (watermark + duplicate-def, conventional-SSA aware post phi-elim) | **C** validator landed in passes/mod.rs + 3 backend hooks | Cheap: gate behind env in run_regression.py |
| 178 | OP-35 | P2 | Post-phi cleanup Case-2 forwarding for Const sources into Value positions could MATERIALIZE the const into a fresh Copy placed at first use instead of refusing (keeps the copy-elimination win for `%fs:16`-style TLS bases) | **C** copy_prop.rs one_round_post_phi filter | Perf polish over the soundness fix |
| 179 | MS-11 | P0 | glibc `make check` triage harness: run subdir test suites under the lccc-built libc with `test-wrapper`, classify {miscompile, unsupported-feature, environment} | **C** this file, session 55 | Next session's main task |

## 10. Session 56 addenda (agent-audit + glibc runtime)

| # | ID | Pri | Item | Evidence | Notes |
|---|----|-----|------|----------|-------|
| 180 | IS-29 | **P0** | Lower `__builtin_fma`/`__builtin_fmaf` (vfmadd at v3) and `__builtin_ia32_cvtpd2ps`; currently emitted as external calls | **M** libm.so link fails (ms178 patch math-use-builtins-fma.h) | Gate for libm.so + make check |
| 181 | LK-23 | P1 | ldconfig `feof_unlocked` undefined | **M** glibc elf/others | same hidden_proto/export family; check libc_nonshared/static path |
| 182 | LK-24 | **P0** | Externally-compiled PIE segfaults at startup against lccc-glibc (glibc's OWN utmpdump/sprof run fine) | **M** /tmp/hg2 SIGSEGV | crt/TLS-init triage with LD_DEBUG under our ld.so |
| 183 | MS-12 | P1 | Audit remaining ~40 `phys_reg_name_32`/`typed_phys_reg_name` sites for XMM-homed operands (8 fixed on evidence this session); consider a debug_assert wrapper | **C** grep audit | loud unreachable!, not silent — fix on evidence |
| 184 | LK-25 | P2 | -r output: spurious blank UND local symtab entry; host libc.so.6/libm NEEDED leak into our libc.so via -lgcc_s | **M** readelf | cosmetic + hygiene |

Agent-patch policy record: Agent C's remat-globaladdr fixes accepted (reworked:
TLS-before-GOT order, get_defining_instruction reuse); Agent B's bare-alloca
accepted; Agent B's MAX_SMALL_INLINE_BLOCKS 12 / -Os cap removal / split-call
`>0` REJECTED AGAIN — do not resubmit without per-workload A/B evidence.

## §11 Session 2026-08-22 (encoding-superiority + glibc-perf session) — carry-over

- **LK-26 (P0, frontend, WRONG CODE)**: struct member offset INCONSISTENCY in
  glibc's `struct pthread` TU (anonymous union { tcbhead_t header;
  void *__padding[24]; } + trailing members). Within ONE compile of
  sysdeps/nptl/dl-tls_init_tp.c three different layouts are used for
  `robust_head`: __builtin_offsetof = 736, the &pd->robust_head VALUE GEP
  = 784, and the pd->robust_head.list STORE-ADDRESS GEP = 1296 **with base
  = Const 0 instead of pd** (emitted: `xor %r12d,%r12d; mov %rax,0x510(%r12)`
  → NULL-page store → every external binary dies in __tls_init_tp).
  Repro: /tmp/tlsinit.i (regen via /tmp/tlspp.sh), probe: /tmp/offprobe.c
  (lccc -O1 -S, read printf immediates: 728/736/704/704 vs codegen 784/1296).
  Synthetic simplification NOT yet found (my 704-byte-union model lays out
  correctly, offsets 1288/1296) — bisect the real TU's include stack; suspect
  duplicate/forward struct-def merging or get_expr_ctype layout cache
  divergence, since three SUBSYSTEMS disagree.
  Blocks: lccc-glibc runtime smoke + perf (ld.so TLS init).
- **PERF-1 (identified, quantified)**: FP intrinsic result path bounces
  through slot + GPR (`vroundsd; movsd->slot; movq slot->rax; movq rax->xmm0`)
  and dead movsd param shuffles: libm_round_family benchmark = 2.99x GCC.
  Fix direction: XMM-home the intrinsic dest / return-value path movsd
  slot->xmm0 direct; kill the entry double-shuffle.
- **PERF-2 (identified, quantified)**: __thread array indexing recomputes the
  TLS base per access instead of GCC's `%fs:sym@tpoff(,%r,8)` addressing:
  tls_seg_access = 2.60x GCC (was 5.8x before emit_seg_*_const_addr).
  Fix direction: TLS-base CSE + tpoff indexed addressing in the backend.
- **EVEX/AVX-512 assembler support** (mathvec blocker, 108 errors when
  --enable-mathvec): zmm/mask-register vmovups/vmovaps forms; big-ticket.
- encdiff status: 10557-insn corpus vs GAS 2.47: LONGER=0, BEATS=180
  (redundant-SIB lea class); probe corpus documents ICC silent
  sign-extension miscompile class (we + GAS 2.47 + GCC + Clang all reject).

---

## §12 Session 65 (2026-08-23) — OpenAI audit verdict + immediate P0 codegen fixes

### 12.1 Verdict on the OpenAI audit

The OpenAI report (`1dc3866…` main snapshot) was reviewed against the current
HEAD `1dc3866…` and its findings cross-checked directly against the code.
Overall assessment:

| OpenAI claim | Verdict | Evidence |
|---|---|---|
| **P0-1: RA scans fat intervals, not segments** (xmltok 12× / inflate 15× stack mem) | **AGREE, already tracked as RA-05** | `regalloc.rs:259–260` builds `scan_ivs` from `liveness.intervals`; `segments` is only used for call-spanning + Tier-2. BACKLOG items RA-05/RA-06 already capture the two pieces. |
| **P0-2: No reload-at-next-use / live-range splitting** (3–5× spill traffic) | **AGREE, already RA-06** | `live_range.rs:26–27` comment verbatim matches the audit ("There is no reload-at-next-use in this module"). `split_ranges.rs` is a correct IR-level call/loop split but not Wimmer-style in-place splitting. |
| **P0-3: FP values still bounce through GPR/Accumulator** (nbody/spectral/struct_copy) | **AGREE in part, PARTIALLY FIXED this session** | `struct_copy` is down from 21.06× → 1.54× (IS-03 64-byte YMM copies + RA-21). Scalar FMA still uses xmm0/xmm1 scratch and a movsd-chain tail (see `float_ops.rs`). FP intrinsic returns still round-trip through stack+rax in places (PERF-1 carry-over from session 56). |
| **P0-4: Vectorizer is pattern-based, not a generic loop vectorizer** (nbody 3.22× / spectral 3.9×) | **AGREE, already OP-05** | `vectorize.rs` header literally says "AVX2/SSE2 vectorization pass for matmul-style loops"; non-reduction FP loops are documented OP-05. Matmul/red-vec are ~1.2–1.49× wins already. |
| **P0-5: FMA is backend-only, not contracted at IR level** | **PARTIALLY DISAGREE** — FMA IS reachable from IR today: (a) scalar mul+add is fused at emit-time through `supports_fused_float_mul_add()` in `generation.rs:3216`, and (b) `__builtin_fma[f]` now lowers to `FmaScalarF32/F64` intrinsics (IS-29 from session 56, landed). What is missing is a robust per-statement contraction boundary (C's "as-if to infinite precision" rule only permits contraction *within* one source expression, not across statements that SROA/GVN make adjacent); the old default was too aggressive and broke nbody FP numerics. This session tightens the default (see 12.2). |
| **P1: No µarch scheduling for Raptor Lake** | **AGREE, IS-21 already open**; port pressure / critical path is not yet modeled. |
| **P1: Peephole operates on assembly text** | **PARTIALLY AGREE** — text peephole is real (x86 `peephole/passes/*` works on emitted assembly), but: (a) `peephole_common.rs` is UTF-8 safe after the MS-09 fix, (b) MachInst path exists and is the intended future home, (c) the backlog already treats peephole as *diagnostic / cleanup*, not as a primary optimization engine (the report's proposed role shift is a good idea but P2, not P0). |
| **P1: Benchmark CI does not protect performance** | **AGREE, MS-01 open**; `ci-bench.py` currently only checks arith/fib/matmul/qsort/sieve and uses a single `fib ≥10×` gate. |
| **"FMA is not in the backend"** | **DISAGREE** — `emit_scalar_fma231{,_with}` and the FMA3 VEX encoding are production in `float_ops.rs` and used for matmul/reduction kernels. GCC/Clang/ICX emit vfmadd for the same shapes that LCCC does under `-ffp-contract=fast`. The OpenAI audit missed this because it looked at default `-O2` (which contracts very conservatively, matching GCC 14). |
| **"Default fp_contract_fast should be on"** | **DISAGREE after empirical measurement on GCC 14.2** — plain `gcc -O2 -march=x86-64-v3` does NOT contract loop-carried reductions (uses `vmulsd`+`vaddsd`), while `-O2 -ffp-contract=fast` / `-ffast-math` does. LCCC's previous default matched `-ffast-math` semantics without the flag, which is a C conformance problem and explains the nbody numeric divergence (59.05 vs GCC -0.17). GCC's gnu* default is `-ffp-contract=on`, which GCC implements conservatively (mostly leaf expressions). LCCC doesn't carry source-location expression boundaries in IR, so the conservative correct default is `fp_contract_fast=false`; users who want FMA across SSA values must pass `-ffp-contract=fast` (as they must on GCC to get loop reductions to contract). |
| **Proposed architecture: "Segmented Greedy Split Allocator (SGSA)" rewrite** | **PARTIALLY AGREE on direction, DISAGREE on approach** — replacing `regalloc.rs` + `live_range.rs` (~6,700 LOC) wholesale is a 4–8 week project with very high regression risk, and the existing codebase already contains most of the required machinery: segments (RA-05), split_ranges (RA-06 foundation), graph-coloring Tier 2 (P0-01b done), hot-loop homes (RA-01/02 done), accumulator/FP homes (RA-23), ABI reg hints (RA-26). The right plan is incremental: make RA consume `segments` as its primary interference representation (RA-05), then wire reload-at-use on top of existing split points (RA-06), then extend split candidates with call/loop/region boundaries. This mirrors how LLVM's greedy allocator evolved from linear scan rather than a clean-slate rewrite. |

Bottom line: **~85% of the OpenAI diagnosis is correct and almost all of it is
already captured in the existing BACKLOG as RA-01..RA-27 / OP-05 / IS-21 / MS-01.**
Two genuine defects were NOT in the backlog and were fixed this session
(§12.2). The one big point of disagreement is methodology: the report
undercounts how much of the "modern greedy RA" machinery LCCC already has, and
its proposed clean-slate rewrite is higher-risk than incremental evolution of
the existing pieces.

### 12.2 Fixes landed this session

| Area | File(s) | Fix | Correctness? | Perf? |
|---|---|---|---|---|
| **Scalar FMA operand move correctness + quality** | `src/backend/x86/codegen/float_ops.rs` | `load_fp_to_reg` used a hard-coded `movq %rax, %xmm1`/`movss %xmm1, <reg>` staging path that clobbered the destination when called for target `xmm1` (used by the FMA emitter). Replaced with a general GPR-selection shim that picks `rax` for `xmm0` targets and `rcx` for everything else, and uses the type-appropriate scalar move (`movss`/`movsd`/`movq`/`movd`) instead of `movaps`. Also fixed an F64-vs-F32 bug where F32 register moves used `movaps` (packed, domain-crossing). Eliminates 1–2 redundant movsd per scalar FMA in the FP emitter fallback path. | **Yes — correctness** (was writing to the wrong XMM in some FMA shapes; no in-tree test tickled it because the XMM allocator currently avoids xmm0/xmm1 homes for non-accumulator FP values, but the ISel path is reachable from `__builtin_fma`). | Small code-size/IPC win on FP-heavy code (libm round family, nbody scalar tails). |
| **Default floating-point contraction policy** | `src/driver/pipeline.rs`, `src/driver/cli.rs` | Changed default `fp_contract_fast` from `true` to `false`; `-ffast-math` still enables it; `-fno-fast-math` now correctly resets to default (off) instead of re-enabling contraction. Updated CLI unit test `fp_contract_is_independent_and_last_option_wins` to reflect the GCC 14.2 parity semantics. This is the FP-contract half of the fix for nbody/spectral divergence (the value 59.05 vs GCC's -0.17 came from over-aggressive FMA merging across separate C statements, which changes rounding order in numerically unstable N-body force calculations). | **Yes — C conformance / correctness parity with GCC** | Pessimizes default-O2 FMA throughput to match GCC; users wanting ICX-style FMA should pass `-ffp-contract=fast` or `-ffast-math` (documented in `scripts/README.md` flow). |
| **FMA regression test flags** | `tests/regression/check_fma_dest_coalesce_codegen.sh` | Added `-ffp-contract=fast` to the structural test that asserts destructive-FMA destination coalescing. This test is specifically asserting that the FMA *coalescing* optimization fires; it must run under the flags that enable FMA formation. | Test fix (test was written under the old over-aggressive default). | — |

Validation:
- `cargo build --profile fastbuild` clean.
- `cargo test --lib --profile fastbuild` → 1101 passed, 0 failed.
- All 8 FMA / FP / reduction shell regression checks pass (`check_fma_dest_coalesce_codegen.sh`, `check_vector_dot_fma_codegen.sh`, `check_multiple_fp_reductions_codegen.sh`, `check_f128_copy_fusion.sh`, `check_ptr_to_float_cast.sh`, `check_load_widen_cast_no_relay.sh`, `check_integer_reduction_vecreg_codegen.sh`, `check_reduction_vecreg_codegen.sh`).
- Full `run_regression.py` corpus: same 18 pre-existing failures as before the patch (arm_matmul_column_stride, arm_vla_struct_varargs_byref, float_direct_slot, fma_chain_accum, fma_zero_multiplier_semantics, fp_param_regalloc, fp_param_wide, loop_promote_affine_alias, memcpy_avx_sse_transition, movb_imm_rex8, nbody, param_home_scratch_reg, regparm3_abi_conformance, session45_vector_recurrence_stress, simd_sse_float, spectral_like_reduction, vectorize_sum_f32, vectorize_trip_count_gate). All of these are pre-existing and outside this session's scope; none were introduced by these changes.
- FMA behavior manually verified on `stmt_test` / `expr_test` / `f` / `dot` kernels against GCC 14.2.

### 12.3 New / updated BACKLOG items from this audit

| # | ID | Pri | Item | Evidence | Notes |
|---|----|---|---|---|---|
| 185 | **RA-05a** | DONE upstream (742ca66) + session 73 | `LiveRange` carries a `SegmentList` of merged live pieces; interference decided on hole-aware pieces via `segments_conflict`; empty segs falls back to the fat model. Phase 2f residual fill generalized to all targets (default ON, ranked: multi-piece values first). The xmltok/inflate 12×/15× stack-mem pathology is addressed at the RA level. | **M/C** xmltok/inflate ratios (re-measure on real TUs). | Sound, incremental — agree with upstream's choice over a clean-slate SGSA rewrite. |
| 186 | **RA-06a** | **P0** | Widen `split_ranges.rs` to support reload-at-use: split a live interval at every use that sits across a high-pressures region, and place a reload immediately before the use rather than keeping the value resident or spilling at the previous def. Start with call-sites only (split around calls, which is the easiest correctness story). | **C/M** `live_range.rs:26` self-documents the gap; RA-06 already open. | Expected -30..-50% spill traffic on gzip/xmltok/inflate once combined with RA-05a. |
| 187 | **IS-29a** | PARTIAL (session 73) | Mixed SSE/Integer eightbyte i128 call results store xmm0/xmm1 directly to their slots (was %rax/%rdx relay). The accumulator-mediated FP call-result path + `__builtin_fma` on non-FMA targets (libm-call fallback) remain. | **M** nbody scalar FMA works (45 vfmadd231sd at -O3 -ffp-contract=fast); libm_round_family 2.99× still open. | The GPR-relay class for struct returns is fixed; the accumulator class needs RA-side XMM homes. |
| 188 | **OP-05a** | PARTIAL (upstream stencils + session 73 map trees) | Upstream 742ca66 added constant-tap stencil loops (affine `a*iv+b` tap offsets off a shared base). Session 73 added elementwise map expression trees (FP Add/Sub/Mul/Div, int Add/Mul, FP sqrt; up to 4 IV-indexed load streams; new VecSub/VecDiv/VecSqrt intrinsics). **Remaining**: nbody `advance` multi-store scatter (fx/fy/fz[i] += ...), spectral's invariant×load dot-product, runtime alias versioning — tracked as OP-05b. | **M** A/B at -O3 v3: nbody 594 insns/159 stk/0 vec vs GCC 366/0/81 (multi-store scatter gap); spectral 212/11/0 vs 151/4/8. | The map-tree + stencil coverage handles elementwise/stencil loops; scatter + cross-stream recurrences remain. |
| 189 | **OP-36** | P1 | Expression-boundary metadata for FP contraction: tag each Mul/Add BinOp with the source expression id (from frontend `ExprId` once FE-03 lands) so that `fp_contract=on` can safely contract within one C expression while still refusing cross-statement contraction. This is what GCC's `-ffp-contract=on` really does; implementing it unblocks "FMA by default at O2" without sacrificing nbody/spectral numerics. | **C/G** GCC parity. | Replaces today's binary on/off with a sound 3-state (off/on/fast) model. |
| 190 | **MS-01a** | DONE upstream (ci-codegen-gate.py) | `.github/scripts/ci-codegen-gate.py` + `ci-codegen-baseline.json`: structural metrics (insns, stackmem, pushes, moves, ymm, fma) for the golden workloads with per-workload tolerance bands; `--update-baseline` for reviewed refreshes. Covers the six golden workloads + stencil5 sentinel. | **C/M** verified FAIL on regressions. | Compiler-Explorer oracle comparison (MS-01b) remains future work (needs networked CI). |
| 191 | **MS-13** | P1 | Split `fp_contract_fast` bool into a tri-state enum `FpContract { Off, OnStmts, Fast }` and thread it through driver → pipeline → passes → backend. Default `OnStmts` once OP-36 lands; today's `false` maps to `Off` for safety. | **C** tech debt from this session's quick fix. | |
| 192 | **IS-30** | SUPERSEDED upstream (4a4520c) | Upstream's exact peephole liveness (real backward dataflow fixpoint over 16 GP families with conservative fallback for unanalysable functions) + copy_coalesce + dead_writes subsume the diagnostic-and-fix role IS-30 targeted. The peephole is now a sound optimization layer, not just cleanup. | **C** OpenAI §10 recommendation addressed by upstream's deeper investment. | Agree with the architectural choice (incremental text-peephole improvement vs MachInst migration) given the soundness of the dataflow fixpoint. |
| 193 | **FE-25** | P1 | Extend `-march=native` (Raptor Lake i7-14700KF) to enable AVX2/BMI/BMI2/F16C/VNNI/GFNI where present and allow the cost model to query the host; today v3 is a hard-coded static feature set. | **C/DESIGN** Raptor Lake is the primary performance target per the LCCC charter §11. | Needed before any real µarch tuning. |
| 194 | **PERF-3** | P2 | TLS base CSE + `%fs:sym@tpoff(,%r,scale)` addressing for `__thread` array accesses (carry-over from session 56 PERF-2, re-confirmed by audit). | **M** tls_seg_access 2.6× GCC. | |
| 195 | **MS-14** | P2 | PMU / `perf stat` harness on the 14700KF (once hardware-access CI is available) that captures cycles, instructions, IPC, branch-misses, L1-dcache-load-misses, LLC-misses, frontend/backend stalls, and the Raptor Lake TopDown metrics, and attaches them to every snapshot in `hotspots/`. The current VM has no PMU, so this is documentation-only until a metal runner exists. | **C** required by charter §10. | |

### 12.5 Session 75 (v3) — general complete unrolling + FP defect hunt

| # | ID | Pri | Item | Evidence | Notes |
|---|----|-----|------|----------|-------|
| 196 | **OP-05b** | **P0** | Vectorize multi-store scatter bodies (nbody `advance`: fx/fy/fz[i] += ...) and computed-invariant dot-products (spectral `A(i,j)`). The dominant remaining FP gap: nbody 594 insns/159 stk/0 vec vs GCC 366/0/81; spectral 212/11/0 vs 151/4/8. | **M** A/B at -O3 v3; runtime 0.26×/0.34× of GCC. | spectral additionally needs non-trapping integer-div proofs + vector int div (no such instruction — needs hoisting the A(i,j) computation out of the vector loop when j-invariant... which it is NOT; the real fix is recognizing A(i,j) as affine in j and synthesizing the vector index math). |
| 197 | **RA-01b** | **P0** | Marching-pointer recurrences (IVSR-created Ptr phis) get slot-homed when RA pressure is high: `leaq bodies(%rip),%rax; movq %rax,344(%rsp)` + per-iteration `leaq 56` + slot reload — 151 of nbody's 159 stack refs are this GPR address traffic. | **M/G** GCC keeps 0 stack refs via direct `bodies+N(%rip)` after full unroll; lccc achieves 66 such folds where the cascade fires. | Extend GlobalAddr remat through Copy chains, or prefer SIB-index forms for IVSR recurrences (base stays a register, index carries the IV). |
| 198 | **UN-01** | DONE | General complete unroller (nested/multi-block constant-trip loops, pure-intrinsic eligibility, const-chain cascade, FP-aware budget). Two miscompiles found+fixed (terminator value rename; phi→Copy init selection). | **M** 1167/1167 unit; 466-corpus unchanged; nbody 5M-step bit-identical to GCC; new unroll_nested_complete.c regression. | The FP budget is load-bearing: unrolling nbody's advance WITHOUT it regresses 594→1183 insns / 159→476 stk (XMM spill wall). |
| 199 | **IS-29a** | PARTIAL | FP benchmark gaps remaining: libm_round_family 288 insns/4 stk/1 vec vs GCC 182/1/24 (accumulator-mediated FP call results). | **M** A/B. | The XMM-home-for-FP-call-results work item remains. |

## 12.4 What the OpenAI report got wrong (and why it matters)

1. **Overstates how little FMA support exists.** The scalar and packed FMA
   emitters are present, tested, and used by the matmul/dot-product kernels.
   What's missing is (a) cross-statement contraction under a sound
   expression-boundary model (OP-36) and (b) routing the result through XMM
   homes for intrinsics like `vroundsd` (IS-29a/PERF-1).
2. **Understates how much of the "greedy allocator" infrastructure is already
   in place.** Tier-2 graph coloring (P0-01b), hole-aware segment liveness
   (partially wired), call-spanning detection, loop-aware hot homes, ABI
   register hints, accumulator/destructive-FMA coalescing, and eviction
   modes 1–5 all exist today. LLVM's greedy allocator was also a 3+ year
   incremental evolution away from linear scan — a clean-slate SGSA rewrite
   would throw away 5 months of carefully validated heuristics.
3. **Asserts that GCC defaults to `-ffp-contract=fast`.** On GCC 14.2
   (Debian 14.2.0-19) it does not, and GCC's own documentation says the
   gnu*-C default is `-ffp-contract=on`, which contracts within expressions
   but not across them. Emitting FMA across separate C statements at
   plain `-O2` is a numeric-stability violation that makes LCCC diverge from
   GCC on nbody and other FP-heavy codes; this session fixes that.
4. **Treats peephole-as-assembly-text as a primary defect.** It is
   sub-architecturally ugly, but it is not a code-quality driver at this
   point. Every hot-path peephole that moves the needle on benchmarks has
   already been pushed into emit-time or ISel; the text peephole is now
   mostly branch inversion, self-move removal, and push/pop compression.

### 12.5 Next-session priorities (recommended order)

1. **RA-05a** — segment-aware interference in the linear scan. Run against
   xmltok/inflate/gzip and track stack-ref counts. Gating metric: xmltok
   stack-mem ratio vs GCC must drop below 6× before merging (target <4×).
2. **RA-06a** — call-site split + reload-at-use for call-spanning live
   ranges. Reuse `split_ranges.rs` SSA rewrite; gate on gzip longest_match.
3. **IS-29a** — XMM home for scalar FP intrinsic returns; kills the
   libm round family 2.99× regression.
4. **OP-05a** — generic non-reduction FP vectorizer skeleton; target nbody
   force calculation first (single outer loop, ~4 inner FP recurrences).
5. **OP-36 + MS-13** — tri-state FpContract + expression-boundary metadata.
6. **MS-01a** — codegen oracle in CI with per-workload tolerance bands.

RA-05a and RA-06a are the biggest single codegen wins on the table (gzip,
expat, zlib, glibc, xmltok, inflate all bottleneck on spill traffic today)
and are the two items the OpenAI audit most strongly supports — but they
must be landed incrementally with kill switches, not as a rewrite.

---

## §13 Session 66 (2026-08-23/24) — P0/P1 execution: RA-05a, IS-29a, OP-05a, OP-36+MS-13, MS-01a

Base: `d35353c` (main tip, verified against GitHub). All items landed with
kill switches, unit tests, and A/B measurements; regression corpus stayed
at the pre-existing 20 failures throughout (431→432 pass: +1 = new
stencil_vectorize regression test).

### 13.1 Landed

| Commit | Item | Summary | Measured |
|---|---|---|---|
| `be097c3` | **RA-05a** (#185) | `liveness.segments` is the scan's primary interference model: `LiveRange::segs` + `segments_conflict` (guarded die-at-birth on the hint path only), coalesce-leader piece/use UNION, `seeded_until` split from `reg_free_until`, hot-escape fallback eviction (ratio 8, fallback-only), last-resort multi-victim sweep eviction (loop-weighted traffic-guarded), segment-aware `verify_handled_history`, segment-aware load→cast fold guard (prologue). Kill switch `CCC_NO_SEGMENT_RA`; `CCC_EVICT_HOT_RATIO=0` disables both eviction extensions. | stack-mem total 370→320 (−13.5%); sqlite_varint 42→11, gzip_crc32 7→0, adler32 78→71; runtime adler 55.1 vs 57.5 ms, hash_table −16%, expat −7% |
| `857ebde` | **IS-29a** (#187) | F32/F64 returns load directly into xmm0 via `load_fp_to_xmm0` — no more `%rax` bounce or stack staging on any FP return (PERF-1 class). | f/g/h/k FP-return shapes all XMM-domain; libm_round_family clean |
| `88d42bd` | **OP-05a** (#188) | Stencil vectorizer: generalized non-reduction FP loops (N taps at constant element offsets from one base, affine-IV byte model `(iv+k)*elem`, single store, alias-safe). Vector body mirrors the scalar expression TREE → bit-exact with NO fast-math; Sub via `Mul(y,-1)+Add` (IEEE-exact); FMA only under Fast. Backend: disp-form `vmovups disp(%base,%idx)` vector accesses, defer-overflow XMM homes for multi-tap values (`CCC_NO_DEFER_OVERFLOW_VECREG`), direct-to-home loads, all-homed 3-operand VEX ops. | fp_memfold_stencil5: 7.8→5.2 ms (gcc 4.8) = 1.08×, checksums bit-identical; n=0..40 trip sweep bit-exact |
| `a41a9b5` | **OP-36 + MS-13** (#189/#191) | `FpContract { Off, OnExpr, Fast }` tri-state threaded through cli→pipeline→passes→backend; frontend tags every FP Mul/Add/Sub with its statement root (`IrFunction::fp_expr_tags`); `detect_mul_add_fusions`/`detect_gap_fma_fusions` pair-gate float candidates. Inlined values are untagged → fail closed. | `-ffp-contract=on` fuses `return a*b+c` (vfmadd) but NOT `t=a*b; t+c` — exact GCC `on` semantics |
| (this) | **MS-01a** (#190) | `ci-codegen-gate.py` + checked-in baseline: per-workload tolerance bands (insns ±2%, stackmem ±5%, pushes ±10%) on gzip_crc32, zlib_ng_adler32, expat_xml_scan, sqlite_varint, glibc_memcmp, hash_table + stencil5 sentinel; wired into bench.yml as a hard gate; ci-bench.py gains not-slower-than ceilings for Arith/MatMul/Qsort/Sieve. Tamper-verified the gate fires. | gate catches a fabricated +137% stackmem regression |
| (this) | cleanup | Removed the duplicated `CCC_LOOP_SPLIT` block in pipeline.rs (verbatim copy-paste junk from a prior session; the second copy ran the opt-in pass twice). | — |

### 13.2 Key engineering decisions (and why)

1. **Segments as under-approximation, not replacement.** The fat model is
   still the fallback everywhere a value has no segment data (synthetic
   vecreg intervals, tests). Every old decision remains representable, and
   `CCC_NO_SEGMENT_RA` restores the exact fat model for bisection.
2. **The die-at-birth waiver is hint-path-only** (`adjacent_ok=true`).
   The first iteration of RA-05a applied it on the rotation path too and
   *immediately* reproduced the documented `or %r9,%r9` sqlite_get_varint
   miscompile — the invariants comment is load-bearing.
3. **Cross-wave occupancy needed its own map.** `reg_free_until` served
   two roles (advisory scalar + hard cross-wave reservation). Segment
   sharing makes in-wave occupancy multi-valued, so the hard reservation
   moved to `seeded_until` and the advisory map became monotone (max).
4. **Sweep eviction is traffic-guarded, not priority-guarded.** The adler32
   starvation (v522, 1200 weighted uses slotted while 110-use holders kept
   registers) was an economic failure, not a ranking failure: the fix
   requires `future_traffic(holders) < incoming_traffic` so every sweep is
   provably a net win. The hot-ratio (8×) version alone was NOT enough —
   same-loop competition made the ratio unreachable.
5. **The load→cast fold guard had to become segment-aware too.** RA-05a
   initially *regressed* adler32 despite better register counts: the fold
   guard's fat-interval test refused folds whenever any holder's HULL
   covered the load point. Fixing the allocator without fixing every
   fat-interval consumer that makes sharing-adjacent decisions would have
   shipped a regression.
6. **OnExpr contraction needs statement identity, not source columns.**
   Expression tags are per-statement ROOTS (bumped per `lower_stmt`), not
   spans: lowering order, inlining, and copy-prop change instruction
   adjacency constantly, but "same statement" survives all of them. UNTAGGED
   values fail closed — pass-created and inlined muls/adds never fuse
   under `on`, which is the GCC-comparable conservative bound.
7. **CI gates measure codegen, not wall-clock.** Shared runners make
   runtime ratios flap; instruction/stackmem/ymm/fma counts are
   deterministic. The runtime ceilings exist but wide; the codegen
   baseline carries the real protection.

### 13.3 Follow-up work

1. **RA-06a (#186) remains open**: Wimmer-style reload-at-use inside the
   scan (this session deliberately shipped the interference half only).
   The sweep-eviction machinery added here is the stepping stone: the
   traffic model is the same one a split decision needs.
2. **nbody force loop** still does not match the stencil shape (two
   stores: `bodies[i].vx -= …` and `bodies[j].vx += …` — the analyzer
   requires exactly one). A multi-store stencil variant (like GCC's
   loop-distribution + SLP) is the natural extension; the alias machinery
   is already in place.
3. **AArch64 stencil** is analyzed but disabled (NEON disp-form loads not
   wired). Wire `VecLoad*(base, off, disp)` in the arm backend.
4. **The vecreg allocator is now segment-aware but still wave-based**;
   long term the Tier-2 graph colorer should consume pieces directly.
5. `hash_table` is −16% runtime with RA-05a but still 17.4s vs GCC — the
   remaining gap is hash-multiply codegen (imul chains), not RA.

---

## §14 Session 67 (2026-08-24) — Agent A/B/C red-team audit + follow-ups

Base: `d35353c` (re-verified = main tip). Every agent patch was applied to a
review branch, executed, fuzzed, and either merged (with repairs), ported (as
ideas), or rejected with measurements.

### 14.1 Audit verdicts

| Agent | Claim | Verdict | Action taken |
|---|---|---|---|
| **A** | `emit_float_binop_into_reg` xorpd-dest-clobber after a coalesced multiply (12 baseline FAILs) | **CONFIRMED REAL MISCOMPILE on main** — dot4 reproducer returns 65 for 70; root cause of 17 of the 20 "pre-existing" regression failures | **MERGED with repair**: the VEX 3-operand/never-xor-live-dest fix is kept; A's `0.0 + rhs → rhs` fold was REMOVED (+0.0 + (−0.0) = +0.0 is signbit-observable and GCC keeps the add); A's SSO bitfield rework REJECTED — measured ZERO observable delta vs baseline in both test shapes (same round-trip AND same wrong wire bytes `00 00 f8 aa` vs GCC `aa f8 00 00`): 20 frontend files churned for an internal representation change that completes nothing |
| **B** | Multi-piece occupancy implemented & rejected (miscompiles adler/switch_dispatch); residual fill ranking clean; OccupancyMap RA-06 infra | **AGREE with both the rejection and the caution.** B's occupancy-rejection independently validates RA-05a's design choice (segments for INTERFERENCE-disjointness, never for occupancy windows). The unwired AllocationMap was correctly NOT shipped | **PORTED** the Phase 2f ranking (multi-piece-first + pressure gate ≥ 8) and — after independent validation (regression w/ VERIFY hard-abort, 300/300 O2/O3 fuzz, byte-identical FP outputs, stack-mem 318→294) — **flipped the default ON for x86-64** (B had left it opt-in; the gated version measured clean twice). Unwired infra not ported |
| **C** | relay/LEA windowed peepholes + kernel corpus; cmov fold designed but NOT shipped (upper-32-bit hazard) | **SOUND — merged as-is.** The deadness contract is genuinely careful: family-wide liveness, width-exact rewrites, ABI-implicit-read refusal at calls/tail-jumps (static-chain r10!), ret reads, div/mul implicit traffic, pinned inline-asm. The xchg hazard is closed (backend never emits xchg outside pinned asm). C's refusal to ship the unsound cmov fold and the label-fallthrough dead-move pass is exactly the right call | **MERGED unchanged** (15-kernel corpus execution verified output-identical vs GCC; PF-07 implemented from C's ledger — see below) |

### 14.2 Follow-up work landed this session

| Item | Status | Detail |
|---|---|---|
| **RA-05a verifier semantics** | **FIXED** | Running `CCC_VERIFY_REGALLOC` over the correctness corpus (never done before!) exposed the eviction handoff being flagged as an overlap: an evicted range's occupancy is HALF-OPEN `[start, cut)` — the incoming takes the register AT the cut — while natural expiry keeps the closed window + guarded die-at-birth waiver. `verify_handled_history` now models both. |
| **PF-07 PIC table-base hoist** | **DONE** | In PIC mode an indexed symbol access re-emitted `leaq sym(%rip), %rcx` per iteration. A GlobalAddr defined at a strictly shallower loop depth than an indexed GEP consumer is promoted out of the never-materialized set → RA homes it → generated once → SIB reads the home (`movl (%r9,%r10,4), %r13d`). crc32k loop: per-iteration leaq eliminated; expat 55.2→50.5 ms (0.98× vs GCC — now a WIN). Kill switch: `CCC_NO_GLOBAL_ADDR_REMAT`. |
| Residual fill default | **DONE** | See B's verdict above. |
| RA-06a reload-at-use | **OPEN (documented)** | The sweep-eviction traffic model + B's occupancy-record design are the stepping stones; a full Wimmer split needs the location map wired into emit. Not riskable in this patch. |
| Multi-store stencil (nbody) | **OPEN (documented)** | nbody's force loop has SIX stores (3× bodies[i].*, 3× bodies[j].*) across two IVs plus field-sensitive load/store disambiguation (x/y/z vs vx/vy/vz). Sound incremental vectorization needs cross-iteration dependence analysis GCC solves by restructuring; skipped deliberately — a miscompile here is worse than the gap. |
| PF-03 cmov fold, PF-04 dead moves, PF-05 adler RA-03, PF-06 isort | **OPEN** | In C's ledger with soundness obstacles recorded. |

### 14.3 Post-merge validation (all with the final tree)

- Regression: **449 pass / 3 fail** (all 3 = missing i686 ELF interpreter on this host; the other 17 former failures were A's xorpd bug)
- Unit tests: **1118** pass (10 new = C's peephole tests)
- Correctness suite: **50/50**; differential fuzz **600/600** at O0/O2/O3/Os; phi-CFG fuzz **360/360**
- Kernel corpus: **15/15** output-identical vs GCC
- Codegen gate: PASS (baseline refreshed — adler 344/56, expat 268/11, sqlite 431/13, crc 192/0)
- `CCC_VERIFY_REGALLOC=1` over the whole regression corpus: clean (after the window-semantics fix)
- Paired runtime vs GCC -O2 (best-of-3, output-checked): wins on expat 0.98×, double_reduction 0.93×, binary_search 0.88×; ties crc/ascii/switch/ring; remaining gaps adler 1.58×, find_bit 1.86×, mandelbrot 1.65×, sqlite 1.33×

---

## §15 Session 68 (2026-08-24) — Agent D audit + rebase + PF-05 root-cause

Base: `57bebcb` (main after the v2 merge; commit `742ca66` is the v2 work).
D's patch was authored against `d35353c`, so its session-73 content is
upstream already; only the session-74 delta is new.

### 15.1 Agent D verdict

| Claim | Verdict | Action |
|---|---|---|
| FP binop dest-aliasing miscompile (`x+1.5`→`1.5+1.5`, dot4 65≠70) | **CONFIRMED** — independently found by D and by Agent A; the fix is already upstream from the v2 merge. Verified D's `fp_binop_dest_aliasing.c` regression test passes on the upstream fix at -O2/-O3: the two fixes are semantically equivalent | D's float_ops variant NOT taken; D's regression test kept (good coverage: constants both sides, float+double, sub/div with dest=rhs-home, signed-zero) |
| 5 flag-lifetime peepholes (producer-retarget, copy+add→lea, copy+shl→lea, setcc/test/cmov, copy+mask→movz) + full-SIB lea fold | **SOUND — MERGED.** The flags model treats unknown mnemonics as readers; `setcc_cmov` KEEPS the cmov (removes the boolean round-trip and inverts the condition), which is why D could ship what Agent C correctly refused (C's variant replaced cmov with add — upper-32-bit hazard); width rules enforce 32-bit-write-zero-extends in both directions; `flags_dead_after` with ret=dead/control-flow=live | Taken wholesale + pass registration; reproduced D's numbers: peephole_ab 7060→6848 (−3.00%) behaviour-identical; kernel corpus 346 vs GCC 264 |
| SSO bitfield patch rejection | **AGREE** — matches my independent measurement exactly (same wrong wire bytes, zero observable delta) | — |
| Residual-fill rejection ("inert, written against an older base, cannot re-measure") | **STALE but reasonable given their constraints** — I had already re-measured it on the merged base (fuzz + VERIFY + outputs) and it is default-on upstream with measurable wins (stackmem −7.5%) | Not reverted |
| PF-07 correction ("crc SIB fold" is impossible; LICM hoisting is the fix) | **AGREE** — and the v2 merge already implements exactly the hoist D describes (`leaq table(%rip)` hoisted; expat/crc improved) | Convergent validation |

### 15.2 PF-05 (adler accumulator spills): root-caused, fix attempted, measured NEGATIVE, reverted

Root cause chain (all measured on k01_adler + zlib_ng_adler32):
- s1/s2 lower to copy webs `{entry, header}` (leaders v11/v15: 4-5 uses,
  def-at-entry, depth 0, prio 3-4) whose in-body arithmetic continuations
  (partial sums, prio 20 each) are SEPARATE scan values. No single range
  carries the recurrence heat; the leaders lose Phase 1 to the partial sums.
- Attempted fix: leader depth = max over group members (ranking-only change)
  + hot_loop_home member-depth qualification.
- Result: static metrics IMPROVED (stackmem 56→54) but runtime REGRESSED
  +28% (57.5→73.8 ms interleaved best-of-3, stable): promoting the
  accumulators displaced the s2 partial sums from the 6-register callee
  pool, and the DO8 body needs the partial sums resident more than the
  accumulator copies homed. k01_adler instruction count also rose 91→98.
- **Reverted.** The real fix is RA-06: the copy web must extend through the
  arithmetic chain so ONE range carries the recurrence and the whole chain
  shares one home. Promoting only the copy leaders shifts the spill.

### 15.3 Honest final standings (interleaved best-of-3, quiet VM)

Methodology note: batch timing on this VM suffers compile-interference;
only interleaved per-program measurement is trustworthy. Earlier session
numbers that beat these are noise-inflated on the GCC side.

| program | gcc | lccc | ratio |
|---|---:|---:|---:|
| gzip_crc32 | 154.6 | 135.2 | **0.87× WIN** |
| binary_search | 0.8 | 0.5 | **0.59× WIN** |
| double_reduction | 117.3 | 113.9 | **0.97× WIN** |
| glibc_memcmp | 5.7 | 5.6 | **0.98× WIN** |
| fp_memfold_stencil5 | 4.8 | 5.1 | 1.08× |
| histogram | 1.2 | 1.3 | 1.07× |
| sqlite_varint | 21.6 | 27.4 | 1.27× |
| expat_xml_scan | 34.3 | 54.4 | 1.59× |
| zlib_ng_adler32 | 36.2 | 58.9 | 1.63× |
| mandelbrot | 892.7 | 1472.5 | 1.65× |

### 15.4 Validation battery (final tree)

1134 unit tests (16 new from D) · 451 pass / 3 env-only fails · 50/50
correctness · 1200+ differential fuzz across O0/O2/O3/Os (all clean) ·
360 phi-CFG fuzz · kernel corpus 15/15 output-identical · peephole A/B
38/38 behaviour-identical.

### 15.5 Open follow-ups (priority order)

1. **RA-06 / PF-05**: arithmetic-chain copy webs (the measured negative
   above is the map of where the naive version fails).
2. **PF-06 isort**: secondary IV for `j+1` (GCC maintains a marching
   register; LCCC recomputes leal+cltq+leaq per iteration) + int-index
   widening pairs. 43 vs 23.
3. **mandelbrot 1.65×**: FP branch-heavy inner loop; nbody-class (the
   multi-store stencil analysis is the known enabler).
4. **expat 1.59×**: hash-multiply chains (imul), not RA.
5. **PF-11** (setcc+test+jne→jCC): no instances in the corpus
   (if-conversion already emits direct branches); implement only when a
   workload shows the shape.

---

## §16 Session 69 (2026-08-24) — Lev Kropp upstream audit + v5

Base: `daf3f48` (main after the v3/v4 merges). Lev Kropp's `levkropp/lccc`
main (76 commits since 2026-08-19, tip `e74883e0`) audited commit-by-commit;
ms178 main already contains the adopted lineage (PRs #209–#223 embed the
backedge-PRE, pointer-compound-assign, DSE, LICM, and cmp/select-fusion
fixes — verified behaviorally).

### 16.1 Audit classification

| Class | Commits | Verdict |
|---|---|---|
| **Semantic fixes already adopted** (2ed29fd8 ptr+=, 1e3d8c84 DSE void-ptr, 53ef5201 LICM use-before-def + vectorizer gates, 40261e3c sqlite ×4, e03da2f1 aggregate init DSE) | ~8 | **Verified present on main by targeted reproducers** (ptr-widening, void-ptr DSE, i-j-k matmul, cmp/select distance) — every Lev bug class I could reconstruct produces CORRECT output on main. His regression tests would pass. |
| **AArch64-only peepholes/RA** (sxtw, stp/ldp, csinc, GDSE, shrink-wrap, x9, madd fusion, ~35 commits) | ~35 | **Not portable to the x86-64 charter target**; the ms178 arm backend diverged long ago. Concepts (function-scoped constant tracking, store sinking) recorded as ideas only. |
| **RA ranking experiments** (priority-first scans, reverse FP phi coalescing, point-precise web merges, c710ed4a/38a85795/6b8fa0b5/b15548ac/eb587035/f958b6af) | ~10 | ms178's RA took a different path (segment-aware interference + Tier-2 graph coloring, both landed and validated). Lev's own roadmap entries mark half of these dead-ends. **No adoption** — merging two RA philosophies without a shared measurement harness is how miscompiles ship. |
| **Portable optimization wins** | | |
| — backedge PRE (a08f6768) | 1 | **Already on main, extended** (main's copy is 672 lines vs his 518 — further developed here). |
| — csinc/csel-increment fold (24200ab4) | 1 | AArch64 csinc has **no x86 equivalent** (x86 cmov cannot add-and-select in one op); the x86 shape is already covered by the flag_peepholes setcc/test/cmov collapse. N/A. |
| — **operand-const hoisting (558e3ed9)** | 1 | **REAL GAP ON MAIN → PORTED & GENERALIZED** (below). |
| — plain-copy loop vectorize (6e955227) | 1 | Main's stencil/map vectorizers already cover `dst[i]=src[i]` (verified: memcpy-shaped loops vectorize). |
| — second-order SR (885f129c) | 1 | Main's univsr covers the triangular-IV shapes; isort's remaining gap is the marching secondary IV (PF-06, open). |
| Docs/roadmap/task notes | ~20 | Read for intelligence; nothing to port. |

### 16.2 The one adopted change, perfected

**x86-64 `int_const_hoist`** (from 558e3ed9's insight): div_by_const's magic
multipliers were re-materialized (`movabsq $2454267027`) inside EVERY loop
iteration; main's pass was AArch64-only because its imm12 model over-hoists
on x86 (the old sieve regression). The port is target-aware:
- x86-64 hoists ONLY out-of-i32-range constants (the movabs class);
  imm32 stays in place (free in cmp/add/imul immediate forms).
- AArch64 keeps the imm12/cmn model; div/rem/mul operand positions
  (`needs_reg`) pay materialization even for small constants there.
- i686 no-op.

Measured: `sum_div7` loop body loses the per-iteration movabsq entirely
(`imulq %r8` reads the preheader home). Gates: 1134 unit tests, 451/3
regression (env-only), 300/300 O2/O3 fuzz, div checksums identical to GCC.

### 16.3 README/CI refresh

- README performance table rebuilt from the canonical runner
  (`run_benchmarks.py`, 30 pairs, 2026-08-24): **geomean 0.8235**
  (conventional-code geomean 1.29 excluding the three algorithmic recursion
  wins). Wins: fib 109×, bitops 1.7×, crc32 1.15×, matmul/double_reduction/
  qsort/ring_fifo/tce_sum all ≤1.0. Honest gap table with root causes.
- `ci-codegen-baseline.json` refreshed (crc 192→174 insns after the
  table-base + magic-const hoists; adler 344→322).

### 16.4 Follow-up status (all sessions consolidated)

1. **RA-06 / PF-05** (adler 1.58×): arithmetic-chain copy webs — the
   measured-negative naive version (§15.2) maps the failure; needs the
   location-map design.
2. **PF-06** (isort/loop_patterns 2.4×): secondary-IV strength reduction.
3. **Non-reduction FP vectorization** (spectral 4.8×, nbody 3.9×,
   mandelbrot 1.65×): multi-store stencil analysis.
4. **expat 1.61×**: hash-multiply chains (imul), not RA.
