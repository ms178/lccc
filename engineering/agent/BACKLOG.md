# Work-item catalog — open items first

Ranked work for **generated-code performance** vs GCC 16.2 / Clang 22.1 /
ICC 2021.10 / ICX. This file is the item catalog (IDs P0 / RA / IS / OP / FE /
AB / PG / LK / MS). The *active* subset lives in
[`../tasks/`](../tasks/README.md); measured negatives and session history
live in [`../DECISIONS.md`](../DECISIONS.md).

Evidence codes: **M** measured LCCC vs GCC on kernels · **G** Compiler
Explorer (`cg162` / `cclang2210` / `cicc2021100` / `cicxlatest`,
`-O2 -march=x86-64-v3`) · **C** tree fact · **S** screening suite.

ROI rank: **P0** this week (testable) · **P1** this month · **P2** after
P0/P1 · **P3** research.

Harnesses: `scripts/kernel_count.py` (per-function counts vs system GCC),
`scripts/peephole_ab.py` (whole-corpus A/B **with behaviour comparison**),
`scripts/godbolt.py` (scoreboard oracle), `.github/scripts/ci-codegen-gate.py`
(structural CI gate).

How to gather data (mandatory):

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

---

## 1. Open items

### 1.0 x86 peephole ledger — PF

| ID | P | Item | Files | Evidence | Accept | Do not |
|----|---|------|-------|----------|--------|--------|
| PF-05 | P0 | Adler accumulator spills — merged into RA-06 (the arithmetic-chain copy-web work); the standalone leader-promotion variant is a measured negative | `regalloc.rs`, `live_range.rs` | **M** adler 1.50× | see RA-06 acceptance | leader promotion without the web extension |
| PF-06 | P1 | isort: secondary-IV strength reduction (marching register for `j+1`) + int-index widening pairs | `iv_strength_reduce.rs`, codegen | **G** isort 43 vs Clang 23 | isort ≤25 insns | recompute chains per iteration |
| PF-15 | P1 | scmp re-sign-extends the same byte three times: "the source is already its own extension" | peephole | **G** scmp 25 vs GCC 12 | designed shape fires | fold without dominance-correct liveness |
| PF-16 | P2 | `movq $imm32,%reg` → `movl`; redundant `cltq` after `lzcnt` | peephole | **G** ffs1 17 vs 11, lz 8 vs 6 | fewer wide imms | change RA homes |
| PF-17 | P0 | Loop rotation hardening → default-enable (the systemic gap) | `loop_rotate.rs`, `ir/mem2reg/phi_eliminate.rs` | **M** ~1 branch/iter suite-wide | see TASK-PF-17 | default-enable before the 15 shapes are root-caused |

(PF-00..04 and PF-07..14 are landed; see §5.)

### 1.1 Register allocation — RA

| ID | P | Item | Files | Evidence | Accept | Do not |
|----|---|------|-------|----------|--------|--------|
| RA-01 | P0 | Remat file-scope `window`/`prev`/`strstart` as `sym(%rip)` / `sym(%rip,idx,1)`, not GOT+stack | `globals.rs`, `live_range.rs`, `generation.rs` | **M** 118 stk; **G** gcc `window(%r9,%rcx)` | match-mini CE: 0 GOT, ≤1 push | PIC/TLS still GOT |
| RA-01b | P0 | Marching-pointer recurrences (IVSR Ptr phis) get slot-homed: 151 of nbody's 159 stack refs are GPR address traffic. Extend GlobalAddr remat through Copy chains; prefer SIB-index forms for IVSR recurrences | `generation.rs`, `regalloc.rs` | **M/G** GCC keeps 0 stack refs via direct `bodies+N(%rip)`; lccc folds 66 where the cascade fires | nbody stack refs 159 → <20 | entry-hoist derived variable-index bases |
| RA-02 | P0 | Keep match IVs (`scan`,`best`,`chain`) in GPR | `live_range.rs` next-use | **M** 248 B frame | stack-mem <20 on gzip `longest_match` | eviction mode 5 global |
| RA-03 | P0 | Adler DO8: the reassoc closed form materialises all eight byte temporaries. Refuse the form above the GPR budget or emit GCC's interleaved prefix-sum tree | `reassoc_accum.rs`, `live_range.rs` | **M** 91 vs GCC 63 insns; prefix-sum = ~26 insns, 0 stack | Adler kernel ≤1.15× GCC | unify incoming/victim cost blindly; another eviction knob (modes 2/4/5 measured worse) |
| RA-33 | **CLOSED — the real spill gap** | Copy/phi webs straddled the 4-byte/8-byte slot width classes (`is_small` requires `!wide_typed`; a Copy/Phi dest is `wide_typed`, the BinOp feeding it is not), so the width guard in `resolve_copy_aliases` rejected every web the CFG coalescer had already proven non-interfering — `arith_loop`: `requested=21 resolved=0 blocked_width_mismatch=21`. Each rejection cost two slots plus a latch copy and turned an in-place `addl %r,slot` into a 3-reference sequence. Fixed by unifying toward the root's wider class. **arith_loop hot loop: 123 -> 62 stack refs, 164 -> 114 insns (GCC: 50 / 131), 41 -> 21 slots. Corpus: -61 kernel stkref, -49 kernel insns, -14 % stkref overall** | `stack_layout/slot_assignment.rs` | **M** see DECISIONS.md | landed | assuming a spill gap is a splitting problem before measuring the traffic's shape |
| RA-06 | P1 — **re-measure before resuming** | The pressure splitter remains correct and off by default. The gap it was built for is now mostly closed by RA-33 (arith_loop 62 vs GCC 50, down from 123), so the premise of both prior negative results has changed. Measure the residual gap's shape FIRST — the last two attempts each spent a session on a cause that turned out to be elsewhere. Cross-block SSA repair is still the prerequisite for any spill-then-color guarantee | `split_ranges.rs` | **M** DECISIONS.md, two negative results | residual gap characterised before code is written | starting from the heuristic again |
| RA-28 | **CLOSED — measured, default stays 3** | `CCC_EVICT_MODE=6` (position-relative, per-use-frequency cost) was measured with `scripts/ra_ab_census.py` over `tests/benchmark/{programs,kernel_corpus}` at -O2 and -O3. Whole-corpus totals look like a win (stkref -1.2 %, rrmov -1.9 %), but the split is the story: **driver (`main`) functions -26 stkref, HOT kernels +17 stkref**. `arith_loop` alone is +15 stkref / +9 insns. An aggregate that weights a benchmark's setup code the same as its kernel is not a performance measurement; the census now reports the split and its verdict is driven by the kernel total | `live_range.rs` `evict_mode` | **M** kernel stkref +17 at both -O2 and -O3 | — | flipping the default on the aggregate number |
| RA-28b | **CLOSED — hypothesis confirmed, but the required splitting is NOT call-splitting** | Measured directly: `CCC_SPLIT_MAX=200` (aggressive call-site splitting) changes **nothing** on the benchmark corpus — 0 insns, 0 rrmov, 0 stkref, 0 push, at both -O2 and -O3. The kernels that regress under mode 6 are **call-free leaf loops**, so `split_ranges`'s call-split can never fire on them. `arith_loop` is the canonical case: 32 simultaneously-live loop-carried ints on a 14-register file, ZERO calls, **165 stack refs vs GCC's 92** (1.79x) and 270 vs 240 insns. Mode 6 loses there because a position-relative cost model evicts more readily and every eviction is a PERMANENT lifetime demotion — and the only thing that fixes it is in-loop pressure splitting, which does not exist | `live_range.rs`, `split_ranges.rs` | **M** SPLIT_MAX=200 delta is exactly zero; arith_loop 165 vs 92 | RA-06 lands, then mode 6 re-measured | assuming the existing splitter is a partial RA-06 — it is orthogonal |
| RA-06 | **P0 — now specified and measurable** | **In-loop, pressure-driven reload-at-next-use.** Scope pinned by the RA-28b measurement: the target is a call-free loop body whose MAXLIVE exceeds the register file, not a call site. Recommended construction (Braun-Hack, CGO 2009, decoupled spill-then-color — NOT LLVM's greedy-with-last-chance-recoloring): (1) compute register pressure per program point; (2) where pressure > k, apply Belady MIN using **global next-use distances** to choose the value to evict — `LiveRange::remaining_cost` and the per-use frequency table already provide the cost side; (3) materialize the spill in the IR as a store at the eviction point and a reload as a FRESH SSA name immediately before the next use, so the value->single-location backend model is untouched (this is why the IR-level formulation is the right one here, and why the volatile-alloca dance is not); (4) the existing scan/Tier-2 colorer then runs on approximately k-colorable IR. Acceptance: `arith_loop` stack refs 165 -> <=100 (GCC parity), gzip and the Adler DO8 loop unregressed, then `CCC_EVICT_MODE=6` re-measured and expected to flip sign | `split_ranges.rs`, `live_range.rs`, `regalloc.rs` | **M** arith_loop 165 vs GCC 92; `ra_ab_census.py` is the gate | arith_loop <=100 stkref AND kernel-total stkref negative | starting from the call-split code path; one-step Wimmer rewrite |
| FE-30 | P1 | **`-std=` now selects `__STDC_VERSION__`** (was pinned at `201710L` for every dialect, so every C23-conditional header took its pre-C23 branch even under `-std=c23`). Landed with the full GCC mapping incl. `iso9899:*` and C2y. Remaining: decide whether the DEFAULT dialect should move to `gnu23` as GCC 15 and Clang 19 have done — that is a semantics change (K&R definitions, `bool`, `nullptr`) and needs its own torture run | `driver/cli.rs`, `driver/pipeline.rs` | **C** landed | default-dialect decision recorded, not guessed | flipping the default dialect without a full torture matrix |
| FE-31 | P1 | **C23 one-argument `va_start(ap)` landed.** lccc injects its own `stdarg.h` macros; `va_start` was a fixed 2-parameter macro, so the C23 form expanded to `__builtin_va_start(ap,)` and failed to parse. Now variadic; `__builtin_va_start`'s lowering already read only `args.first()`, so both forms lower identically. Unblocks `gcc.c-torture/execute/pr117432.c`, which the host GCC 14.2 cannot compile at all | `preprocessor/includes.rs` | **C** landed | — | evaluating the second argument (C says it is not evaluated) |
| IC-05 | P1 | **if-conversion speculation is now dereferenceability-based**, replacing a shape heuristic ("IV-addressed inside a loop") that speculated loads the scalar never performs. Adopted from Agent Z and hardened here: coverage now compares access **extent** (a 1-byte covering deref no longer licenses an 8-byte speculated load) and the address key no longer collapses **truncating** casts. Remaining: `sink_conditional_stores` uses the same key — audit it against the same two properties | `passes/if_convert.rs` | **C** landed; see `AUDIT-2026-09-05-agentz-followup.md` | store-sink audited against extent + cast injectivity | re-introducing a shape-based gate |
| RA-32 | P2 | **FP copy-web coalescing** (`fp_copy_web_groups`) merges copy-connected scalar-FP webs so a reduction's combine→carry handoff stops bouncing through a slot. Adopted from Agent Z; its interference validation now fails **closed** on members with no recorded segment (it silently skipped them, i.e. "no data" read as "no interference"). Remaining: the merged leader's interval is the fat `[min start, max end]` while `attach_scan_segments` only sees the leader's own segments — verify the scan cannot place another value in a member's live hole | `backend/regalloc.rs` | **C** landed | leader segment coverage proven or made conservative | widening the web admission before that proof |
| VEC-10 | P1 | **Integer map lane ops landed** (`VecSub/And/Or/Xor I32x4|x8`, `VecSubI64x2`). Verified emitting `vpsubd`/`vpand`/`vpor`/`vpxor` on the corpus and hash-identical to GCC at -O0/-O2/-O3/-Os and on i686. Remaining: `VecSubI64x2` (`psubq`) is wired and correct but never selected — the 2-lane I64 map path does not admit the shape; find and widen the gate, or delete the op if the shape is unreachable | `passes/vectorize.rs`, `x86/codegen/intrinsics.rs` | **C** psubq count 0 on the corpus | psubq fires or the op is removed | leaving a wired-but-dead lowering |
| VEC-11 | P1 | **All four strict min/max ternary forms now fold.** The missing one was `l < r ? r : l` -> `max(r, l)` — the clamp HEAD `x < 0 ? 0 : x`. With it, `x<0?0:x; x>1?1:x` lowers to `vmaxps`+`vminps` (4-instruction vector body); **GCC 14.2 does not vectorize that loop at all**. The exactness argument is operand ORDER: MIN/MAX return src2 on every special case (NaN, both-zero), so a strict ternary folds iff its FALSE arm is src2 | `passes/vectorize.rs` | **C** landed, unit + differential tests | — | folding non-strict `<=`/`>=` (they differ on +-0, and `<= ? r : l` on NaN) |
| VEC-12 | P0 | **Loop-carried value other than the IV is now rejected** in BOTH the map and stencil analyzers (fail-closed). Was a live miscompile: `for (i) { d[i]=a[i]*2; acc+=3; } return acc;` returned 3*(n/8) — 0 at n=1. Rejecting it in the map analyzer alone just handed the shape to the stencil analyzer, which miscompiled it identically. Remaining: re-admit these shapes by RECONSTRUCTING the secondary affine recurrence in the epilogue (`acc0 + n*step`, `base + n*stride`) instead of refusing them — a real vectorization opportunity currently left on the table | `passes/vectorize.rs` | **C** landed | secondary affine recurrences reconstructed, differential-clean | re-admitting without an epilogue reconstruction |
| CG-07 | P1 | **`-0.0` in static data no longer collapses to `.zero`.** The array emitter used `is_zero()` (C truthiness, under which -0.0 IS zero) to build `.zero N` runs, so `static const float t[]={-0.0f}` read back as +0.0f and `1/t[0]`, `signbit`, `copysign` and MINPS/MAXPS lane selection all observed the wrong value. New `IrConst::is_all_zero_bits()` is the object-representation predicate. Remaining: audit the other `is_zero()` call sites in emitters for the same confusion (the NULL-pointer test in `const_eval_global_addr.rs` is correct as-is) | `ir/constants.rs`, `backend/common.rs` | **C** landed, regression test | emitter `is_zero()` audit complete | using `is_all_zero_bits` where C truthiness is meant |
| RA-29 | P1 | Feed **rematerializability** into `spill_cost_at`: a constant or `GlobalAddr` def costs almost nothing to spill and the backend already rematerializes them (`CCC_NO_GLOBAL_ADDR_REMAT`). Add `RematKind` from the defining instruction and scale the cost. Direct input to RA-01/RA-01b | `live_range.rs`, `globals.rs` | **C** every value priced identically per weighted use | match-mini GOT count, nbody stack refs | pricing remat at zero (the reload still costs an instruction) |
| RA-30 | P2 | Switch `calculate_spill_weight`'s denominator from the fat envelope to `occupied_points()` (`occupancy_len` is now maintained by `set_segments`). One line, but it changes mode-3 ranking, so it needs its own A/B round | `live_range.rs` | **C** a huge envelope with tiny coverage looks cheap to spill | isolated A/B, no regression | bundling it with the RA-28 measurement |
| RA-31 | P2 | Per-use **cost classes**: a use that folds into a memory operand is far cheaper to spill than one needing a reload into a register (SIB base, shift count). `folded_index_uses` / `never_materialized` already carry most of the information; feed it into `use_weights` (LLVM `mayFoldMemoryOperand`) | `regalloc.rs`, `live_range.rs` | **C** all uses weighted alike | census `stkref` | inventing fold rules the emitter does not implement |
| RA-07 | P1 | Call-site caller-saved save vs always r12 | `regalloc.rs` `caller_save_spans` | **C** Phase 2b slots | use spans more; prologue stk down | promote r11 to callee-saved |
| RA-08 | P1 | Affinity coalescing overlapping copies | `regalloc.rs` `build_coalesce_groups` | **C** disjoint-only | fewer movs, no extra spills | merge overlapping intervals |
| RA-09 | P2 | Residual Briggs on spilled | new | **C** | fewer reloads | rename the slot colorer as a GPR allocator |
| RA-10 | P2 | GPR spill to XMM | new class | ICX/ICC | gzip gate | clobber SSE ABI |
| RA-11 | P1 | One `RangeMetadata` per function | `live_range.rs` | **C** walk × waves | compile time | change eviction semantics |
| RA-14 | P2 | Stronger `split_ranges` (phis, multi-exit) — call-split already works | `split_ranges.rs` | **C** fail-closed | more splits, gzip gate | mem2reg on split slots |
| RA-15 | P1 | Param group priority remaining holes | `bump_coalesce_group_priority` | **C** adler/memcmp history | fewer param spills | break SysV arg regs |
| RA-16 | P2 | Recalibrate PGO RA weights after RA-01 | `pgo_weight_max` | **C** +4.7 % gzip at 4 | gzip + kernels | raise max first |
| RA-17 | P1 | Folded GEP **base** liveness x86 remaining | `prologue.rs` `collect_gep_fold_base_links` | **C** gz_reset NULL-store | no NULL-store miscompile | extend folded **indices** on x86 |
| RA-19 | P1 | PhysReg map table in one file | `emit.rs` vs comments | **C** r10/r11 swap history | single source of truth | silent renumber |
| RA-20 | P2 | i686 eax/ecx model → clobber oracle from emit | `regalloc.rs` 2d/2e | **C** | i686 tests | copy x64 policy |
| RA-22 | P3 | PBQP irregular (AArch64) | — | after RA-01..06 | — | before x64 codecs win |

### 1.2 x86 ISel / peephole / MachInst — IS

| ID | P | Item | Files | Evidence | Accept | Do not |
|----|---|------|-------|----------|--------|--------|
| IS-02b | P1 | Field-store/arg halves of the SSE-class ABI (returns are DONE: xmm0/xmm1 for all-SSE 16-byte structs) | `calls.rs`, `float_ops.rs` | **M** mk 24→11 insns (returns) | mov count ≤40 for struct shapes | keep rax shuttle |
| IS-05 | P1 | Reduction FMA folds one proven source stream as a memory operand; strict non-contracted `vmulps/pd` still round-trips the other `VecLoad` through a temporary slot | x86 vector emitter | **M/G** FMA shape complete; strict p17/p18 retain one store + stack operand | direct stream memory operand without alias/liveness regression | fold aliased |
| IS-07 | P1 | `cmov` limit idiom | `if_convert.rs` | **G** gcc match `cmovc` | cmov in match | if-convert huge blocks |
| IS-08 | P1 | Load-cast fold remaining | `prologue.rs` load_cast | **C** 33 % gzip-9 was movzbl | leftover movzbl down | disable the fold globally without measure |
| IS-09 | P1 | Cmp-replay only from **slots** | keep | **C** sqlite `yy_shift` | no sqlite SEGV | replay from physreg |
| IS-10 | P1 | Flag fusion Copy/Cast chain — extend carefully | exists | **C** expat accounting SEGV if multi-use | more fused cmp | multi-use flags |
| IS-11 | P1 | C `__ffs` if-tree → gcc **`andn`+`cmov` chain** (not tzcnt) | `bit_idioms.rs`, `alu.rs` | **G** gcc cmov×11; 1.51× gap | find_bit CE vs gcc | blindly emit tzcnt |
| IS-13 | P1 | ALU+mem for spilled | peephole | ICX lookalike | fewer reloads | change RA homes |
| IS-16 | P2 | Match inner 258 B with `pcmpeqb` | vectorize | spike | gzip gate | ship without CRC/adler oracles |
| IS-17 | P1 | MachInst profit model ≠ 32-inst cutoff | `prologue.rs` | **C** gzip −3 % | beat gzip then raise | `CCC_MI_FORCE_LOOPS` |
| IS-19 | P2 | Text peephole vs MachInst conflict documented | `CCC_NO_MACHINST` | **C** | documented interaction | dual-apply same fold |
| IS-21 | P2 | Scheduler already lost gzip 3 % — µarch model for Raptor Lake | emit / prologue | **C** `-mtune` diagnostic only | PMU after ISel | turn on as default |
| IS-22 | P1 | Indexed GEP SIB completeness | generation + memory | **S** loop_patterns 1.85× | SIB in loops | GOT for file-scope |
| IS-23 | P2 | `repe cmps`/`pcmpeqb` memcmp | | noisy <20 ms | scale work first | optimize 20 ms kernel |
| IS-25 | P2 | 3-channel mul ILP (`exclude_every_third_mul_temp`) document or kill | `regalloc.rs` | **C** magic | documented or gone | silent heuristic |
| IS-26 | P1 | GlobalAddr fold vs GOT (`needs_got_for_addr`) | never_materialized | **C** | RIP for file-scope | PIC/TLS |
| IS-27 | P2 | i128 through rdx:eax without excluding r8–rsi whole fn | prologue i128 filter | **C** | more GPR | ABI break |
| IS-29a | P1 | Remaining FP call-result class: accumulator-mediated results + `__builtin_fma` on non-FMA targets (libm-call fallback). Direct xmm0 returns are landed | `float_ops.rs`, `calls.rs` | **M** libm_round_family now 0.411× (win); the class remains for mixed shapes | no rax/stack bounce on FP returns | reintroduce the relay |

### 1.3 Optimizer / alias / SROA / vectorize — OP

| ID | P | Item | Files | Evidence | Accept | Do not |
|----|---|------|-------|----------|--------|--------|
| OP-02 | BLOCKED | Dominator-aware SROA copy-out stays default-off (miscompiles `structs_bitfields`) | aggregate_sroa | **M** | require dominance/ABI proof + full fuzz | enable raw flag |
| OP-03 | P1 | SROA cross-block memcpy (same-block only now) | same | **C** | more scalarized fields | ignore dominance |
| OP-05b | P0 | **Multi-store scatter vectorization** (nbody `advance` 6 stores, 2 IVs, field-sensitive disambiguation) + computed-invariant dot (spectral `A(i,j)` affine in j → synthesize vector index math). nbody 594/159/0 vs GCC 366/0/81; spectral 212/11/0 vs 151/4/8 | `vectorize.rs` | **M** runtime 0.26×/0.34× of GCC | ymm counts; checksums bit-identical | unsound cross-iteration dependence guesses |
| OP-07 | P1 | Runtime alias versioning | none | **C** | versioned loops, checksums | version everything |
| OP-08 | P1 | Nested IVSR with dependence | `iv_strength_reduce.rs` | **C** nested-matmul wrong if forced | loop_patterns + matmul | force overlapping |
| OP-09 | P2 | Enable `univsr` after SSA repair | gated | **C** | | enable gated pass raw |
| OP-10 | P1 | Unroll vs RA pressure | `loop_unroll.rs` | **C** | gzip unregressed | unroll longest_match blindly |
| OP-11 | P2 | Loop interchange matmul jk | vectorize / loop | **S** | matmul vs ICX | interchange dependent |
| OP-13b | P2 | DSE cross-block extension (same-block landed) | `dse.rs` | **C** pointer-phi escape boundaries — stay conservative | dead cross-block stores; alias guard pinned | re-open with a purely local read-set argument |
| OP-14 | P1 | If-convert >8 inst / more diamonds | `if_convert.rs` | **C** | more cmov | convert huge blocks |
| OP-16 | P1 | `bit_idioms.rs` coverage (popcnt, andn, ffs) | bit_idioms | **G** | IS-11/12/28 fire | |
| OP-17 | P2 | `reassoc_accum.rs` vs FMA | reassoc | | FMA-friendly assoc; Adler prefix-sum tree (RA-03) | break fp assoc at -O2 default |
| OP-18 | P2 | `fp_const_hoist` / `int_const_hoist` audit | | **C** | | |
| OP-19 | P1 | `load_forward` / `store_load_forward` vs GVN overlap | | compile-time | no double work | |
| OP-20 | P1 | `loop_memory_promote` more consumers | alias.rs | **C** | more promotions | ignore alias |
| OP-21 | P2 | `quadratic_sr` | | | | treat wraparound as bit-identical (it is not under -fwrapv) |
| OP-22 | P1 | `redundant_loads` follow-ups | | | | |
| OP-23 | P2 | `block_layout` vs PGO conservative | | **M** expat 131→248 | don't reorder | layout reorder hot loop |
| OP-24 | P2 | `outline_switch` threshold 40 — measure size vs gcc | `outline_switch.rs` | **C** | size vs gcc | set 999999 |
| OP-25 | P1 | Inline `name_cont` | `inline.rs` | **G** gcc inlines | expat scan | 189-inst varint spike |
| OP-26 | P1 | Pressure-aware inline | inline.rs | sqlite | no +1119 B 1.00× | |
| OP-27 | P2 | IPCP function cloning | `ipcp.rs` | | | clone explosion |
| OP-28 | P3 | SCEV full (subsume IVSR+alias+licm) | | high risk | | rewrite all three at once |
| OP-29 | P1 | Pass order experiment with gzip/adler/expat oracles | `src/passes/mod.rs` | **C** dirty skip | measured order | shuffle without oracles |
| OP-30 | P2 | Keep `-O0` vs `-O2` distinct | `src/passes/README.md` | **C** tiers real | tests at both | assume all -O identical |
| OP-32 | P1 | restrict-param `forms_disjoint` wiring into GVN (symbol-based GEP keys done) | `gvn.rs`, `alias.rs` | **M** cross-site forwarding fires | fewer redundant loads | CSE aliased |
| OP-33 | P2 | `segments`-based dead-store elimination | `dce.rs` | **C** `segments` exists | dead stores gone | DCE volatile |
| OP-36 | P1 | Extend expression-boundary contraction: `FpContract::OnExpr` exists; widen tag coverage (inlined/hoisted values fail closed today) | `lowering`, `pipeline.rs` | **C/G** GCC `on` parity | more OnExpr fusions, zero cross-statement fusions | fuse untagged values |

### 1.4 Frontend / IR — FE

| ID | P | Item | Files | Evidence | Accept | Do not |
|----|---|------|-------|----------|--------|--------|
| FE-02 | P1 | TBAA / `restrict` → IR attrs; per-pointer-level qualifiers (also unblocks the const-lvalue diagnostic) | IR attrs | **C** noalias grep empty | attribute on IR | pretend alias is TBAA |
| FE-03 | P2 | ExprId not pointer | lowering | **C** TODOs | stable ids | |
| FE-04 | P2 | Vector subscript | frontend | **C** | | |
| FE-05 | P2 | typeof diagnostic | | **C** | | |
| FE-06 | P2 | `__builtin_nan` payload | | **C** | | |
| FE-07 | P2 | Parser panic-mode | ideas | | | |
| FE-08 | P2 | `_Complex long double` | | **C** | | |
| FE-09 | P2 | Unsigned subint IR | `ideas/fix_ir_unsigned_subint*` | **C** | | |
| FE-10 | P1 | Alignment/range on IR for RA | IR | **C** | better homes | |
| FE-11 | P2 | `fastcall` fptrs | | **C** | | |
| FE-12 | P2 | Nested sizeof | | **C** | | |
| FE-13 | P3 | Use-def shared context | ideas high_use_def | **S** | O(1) use | rewrite every pass at once |
| FE-14 | P2 | Builtin table unify sema/type_checker | | **C** | | |
| FE-15 | P2 | C11 UCN lexer | | **C** | | |
| FE-16 | P1 | Volatile through peephole (phase 2: volatile-qualified casts/MMIO, struct fields, `int * volatile`, aggregate initializers) | prologue `LCCC_VOLATILE_SLOT` | **C** DECISIONS volatile section | volatile preserved | fold volatile |
| FE-17 | P2 | va_arg_pack stub | | **C** | | |
| FE-18 | P2 | Dependency file `-M` only source | pipeline TODO | **C** | | |
| FE-19 | P2 | `__builtin_cpu_supports` runtime CPUID (allowlist is landed; runtime detection is the remaining piece) | `expr_builtins.rs` | **C** | runtime CPUID on non-v3 targets | re-enable the "return 1" SIGILL bug |
| FE-20 | P2 | `usual_arithmetic_conversion` edge-case regression tests (core fix landed) | `common/types.rs` | **C** | edge-case tests | re-enable the `1LL + 1UL` bug |
| FE-23 | P1 | Address-space qualifiers on function PARAMETERS and multi-level positions | sema | **C** decl-level only | glibc THREAD_* macro coverage | |
| FE-24 | P2 | `#error` message verbatim (no macro expansion) | diagnostics | **M** dl-trampoline.h | GCC-parity text | |
| FE-25 | P1 | Real `-march=native` for Raptor Lake (14700KF): enable AVX2/BMI/BMI2/F16C/GFNI/VNNI where present; cost model queries the host | `driver`, `codegen options` | **C** v3 is hard-coded static today | detection on the 14700KF | pretend v3 == native |

### 1.5 ABI / aggregates / stack — AB

| ID | P | Item | Files | Evidence | Accept | Do not |
|----|---|------|-------|----------|--------|--------|
| AB-01 | P0 | SysV SSE-class args in XMM not rax shuttle (returns landed) | ABI + float_ops | **S** struct_copy 1.35× | XMM class | rax roundtrip |
| AB-02 | P0 | sret + one wide copy | ABI | same | one ymm/xmm copy | field loop |
| AB-03 | P1 | 4-byte slots without zext bug | stack_layout README | **C** | | |
| AB-04 | P1 | Overalign param copy already special-cased; audit | prologue Alignas(32) | **C** | | |
| AB-05 | P1 | Parallel copy param prestores remaining | prologue gpr_prestores | **C** | fewer cycles | |
| AB-08 | P2 | ARM KVM VA layout | ideas | **C** | | |
| AB-10 | P2 | Postgres frame size | ideas | **C** | | |
| AB-11 | P1 | Alloca escape analysis more GEP | stack_layout | **C** | | |
| AB-12 | P2 | Two coalescers unify | RA vs stack | **C** | one policy | |
| AB-15 | P1 | Dead XMM save in integer varargs remaining | vararg_fp_save | **C** | | |

### 1.6 PGO / IPO / layout — PG

| ID | P | Item | Files | Evidence | Accept | Do not |
|----|---|------|-------|----------|--------|--------|
| PG-01 | P2 | Sample PGO | ideas high_sample | **C** | | |
| PG-02 | P2 | Loop-versioned devirt | ideas | **C** | | |
| PG-04 | P1 | RA use block counts after RA-01 | pgo_weight | **C** | gzip gate | `PGO_WEIGHT_MAX>1` first |
| PG-05 | P2 | Value specialization | ideas | **C** | | |
| PG-06 | P1 | Keep conservative layout | | **M** expat | no 131→248 | reorder hot loop |
| PG-07 | P2 | LTO | none | | | |
| PG-08 | P1 | Force-inline budget vs frame | | **C** 8B/SSA | sqlite spike | |
| PG-09 | P2 | Switch profile density vs bt | switch_dispatch 1.40× | **S** | | |
| PG-10 | P2 | Function placement | | | | |
| PG-11 | P1 | Flat-profile gate keep | integrated PGO | **M** adler −17 % pre-fix | keep `has_spread` | drop gate |
| PG-12 | P1 | Don't promote 95 % single-target | `LCCC_PGO_PROMOTE_STABLE=95` | **M** +28 % if promoted | keep | promote stable |

### 1.7 Linker / assembler / multi-arch — LK

| ID | P | Item | Files | Evidence | Accept | Do not |
|----|---|------|-------|----------|--------|--------|
| LK-01 | P2 | mmap I/O (biggest remaining link-speed lever; `Vec→Arc` measured always-copy) | ideas | **C** | | per-access Deref re-slicing |
| LK-02 | P2 | `--icf` (byte-only identity is unsound; whole-link absolute-reloc scan required) | linker | **C** | | ship unsafe ICF |
| LK-03 | P2 | RISC-V PIC auipc | **C** | | | |
| LK-05 | P2 | RISC-V dash FAIL — failure mode (compile vs runtime) unknown; needs qemu-user + riscv64 sysroot first | current task | **C** | reproduce, classify, fix | guess without the environment |
| LK-06 | P2 | RISC-V kernel boot hang | ideas | **C** | | |
| LK-07 | P2 | `-mno-sse` full | mod.rs TODO | **C** | | |
| LK-08 | P2 | Encoder completeness | `encoder/mod.rs` TODO | **C** | | |
| LK-09 | P2 | i686 archive merge | TODO | **C** | | |
| LK-10 | P2 | R_X86_64_32 PIC | TODO | **C** | | |
| LK-11 | P2 | RISC-V `@cc` | TODO | **C** | | |
| LK-12 | P2 | RVV masked | TODO | **C** | | |
| LK-13 | P2 | Multi-alt asm ARM/RISCV | TODO | **C** | | |
| LK-14 | P1 | `encdiff.py` / `insndiff.py` in the RA ISel loop (10557-insn corpus exists: LONGER=0, BEATS=180) | scripts | **C** | wired into A/B | manual eyeballing |
| LK-15 | P2 | ELF32 script setup.elf | ideas | **C** | | |
| LK-16 | P2 | `--emit-relocs` | | | | |
| LK-18 | P3 | QEMU build | ideas | **C** | | |
| LK-19 | P0 | **IFUNC end-to-end**: `__attribute__((ifunc))` sema/lowering, `.type %gnu_indirect_function` data refs, `R_X86_64_IRELATIVE` for data words, ld.so semantics | sema, lowering, linker | **M** glibc configure rejects `--enable-multi-arch` (probe: `.quad ifunc-sym` produced no IRELATIVE; attribute produced no IFUNC symbol) | multi-arch glibc configure; IRELATIVE in probe | weaken the glibc capability gate silently |
| LK-20 | P1 | `-r` output emits a spurious blank `UND NOTYPE LOCAL` symtab entry | emit_rel symbol table | **M** `readelf -sW` | no bare `U ` in GNU nm | |
| LK-21 | P1 | `-l` library resolution in the driver's `-r` path (`-lgcc` dropped) | common.rs | **C** | reloc-link finds libgcc | |
| LK-22 | P2 | `-Map` for exec/shared links from the `-r` spellings | mapfile.rs | **C** | full map output | minimal member list |
| LK-23 | P1 | ldconfig `feof_unlocked` undefined | glibc elf/others | **M** | hidden_proto/export family resolved | |
| LK-24 | P0 | **Externally-compiled PIE segfaults at startup** against lccc-glibc (sub-bugs fixed; the custom-interpreter smoke still SIGSEGVs 139) | crt/TLS-init triage | **M** LD_DEBUG under our ld.so | smoke exits 0 | weaken the smoke gate |
| LK-25 | P2 | `-r` blank UND entry + host libc NEEDED leak via `-lgcc_s` | readelf | **M** | clean readelf | |
| LK-27 | P2 | EVEX/AVX-512 assembler forms (zmm/mask registers) — the mathvec blocker (108 errors) | encoder | **M** glibc --enable-mathvec | mathvec compiles | |
| LK-28 | P0 | objtool interop: `.discard.annotate_insn` empty-data-with-relocations assembler bug + `find_insn` decode mismatch; then the real `vmlinux` lccc-ld link, ORC generation, QEMU boot smoke | assembler, linker | **M** boot chain blocked after 120 `.o` + 14 archives | objtool clean on lccc objects | skip objtool silently |
| LK-29 | P1 | Statement-expression + inline-asm type inference (blocks all of `net/*`; minimizer + next steps recorded in git history session 83) | sema | **M** net/*.o fail | net objects compile | suppress via warning (root-cause it) |
| LK-30 | P2 | Missing x86 system instructions: VMX (`vmcall` C1, `vmlaunch` C2, `vmresume` C3, `vmxoff` C4, `vmfunc` D4), SVM (`clgi` DC, `stgi` DD, `invlpgb/invlpga` DE/DF), `pcommit`, `clwb`, `clflushopt`, `tlbsync`, `tpause`, `umwait`, `waitpkg` | `encoder/mod.rs`, `registers.rs` | **M** kernel make | assembler accepts; encodings round-trip | accept silently wrong encodings |

### 1.8 Measurement / process / compile-time — MS

| ID | P | Item | Files | Evidence | Accept | Do not |
|----|---|------|-------|----------|--------|--------|
| MS-01 | P0 | Compiler-Explorer oracle comparison in CI (networked oracles; the structural `ci-codegen-gate.py` is now wired — the CE delta half remains) | `scripts/godbolt.py` | procedure §1 | CI artifact | skip oracles |
| MS-03 | P1 | CI correctness+regression+fuzz smoke beyond the lib tests | `.github/workflows` | **C** | suites run in CI | |
| MS-04 | P1 | OnceLock env knobs break parameterized tests | `live_range.rs` | **C** | tests independent | |
| MS-05 | P2 | Compile-time: pass rescans | use-def FE-13 | **C** | | |
| MS-06 | P1 | Document PhysReg map once (RA-19) | | **C** | | |
| MS-08 | P1 | Consolidate 50+ `CCC_` env vars into a `RaConfig` struct | `regalloc.rs` (29), `live_range.rs` (7), `prologue.rs` (16) | **C** mixed polarity | single source of truth | env vars in hot path |
| MS-09 | P1 | Close the peephole UTF-8 audit: verify no shipped binary carried corrupted asm from the old `bytes[i] as char` | `engineering/evidence/` | **C** old code corrupted UTF-8 | audit report | re-enable `bytes[i] as char` |
| MS-10 | P1 | `CCC_VALIDATE_SSA` in CI for the regression corpus (watermark + duplicate-def post phi-elim) | run_regression.py | **C** validator landed | gate behind env in runner | |
| MS-11 | P0 | glibc `make check` triage harness (classify {miscompile, unsupported-feature, environment}) — promised in session 56, never built | new harness | **C** | harness + first triage table | ad-hoc triage |
| MS-12 | P1 | Audit remaining ~40 `phys_reg_name_32`/`typed_phys_reg_name` sites for XMM-homed operands; consider a `debug_assert!` wrapper | grep audit | **C** 8 fixed on evidence | loud unreachable, never silent | silent fallback |
| MS-13 | P2 | `FpContract::OnExpr` default once OP-36 tag coverage is broad | driver | **C** tri-state landed, default Off | default flip measured | flip without the tag audit |
| MS-14 | P2 | PMU / `perf stat` harness on the 14700KF: cycles, IPC, branch-misses, L1/LLC misses, TopDown; snapshots into `engineering/evidence/pmu/` | new harness | **C** no VM PMU | metal-runner evidence | VM claims labelled hardware |

---

## 2. Architectural flaws (read before coding)

**A. One codegen, one RA — with a dual ISel residue.** The dead MachInst RA
module is deleted (P0-01a); the MachInst ISel/emit path piggy-backs on the
main RA via `resolve_stack_vregs` and is gated off on large loops
(IS-17). Unifying ISel paths without a cost model repeats the gzip
regression.

**B. Location model is converging.** `ExplicitLocation::{Reg,Accumulator}`
owns non-stack scalar homes; `SlotAddr::Reg` is production across all
backends. Remaining debt: accumulator legality still partially derives from
consumer order (RA-23 tail), vector/MachInst locations.

**C. RA interference is segment-aware; splitting is not.** `liveness.segments`
drives interference (RA-05a) but the scan still demotes whole lifetimes —
no reload-at-next-use (RA-06). `split_ranges.rs` is a correct IR rewrite,
not Wimmer-style in-place splitting.

**D. Accumulator is still the ISA on some FP paths.** Aggregate scalar
copies and mixed-ABI shapes stage through GPRs in places (IS-29a, AB-01).

**E. Sema enforces core constraints.** Remaining frontend work: per-pointer
qualifiers (FE-02/TBAA), canonical anonymous aggregate identity (FE-03/04).

**F. Alias is loop-SCEV-lite.** `alias.rs` drives LICM, redundant_loads,
LMP; restrict-param wiring into GVN remains (OP-32).

**G. PGO remains conservative by design.** `CCC_PGO_WEIGHT_MAX` default 1;
layout must not reorder (expat 131→248 ms).

**H. ExprId is a pointer** (FE-03) — const-eval/typeof suffer.

**I. Lifetime demotion is the only spill model** — RA-06.

## 3. Empirical scoreboard (do not "fix RA" for CRC)

Canonical screening: `-O2` paired medians vs GCC 14.2, checksums verified,
shared 2-core VM (no PMU). Full table in root `README.md`
(2026-08-28, geomean ≈ 0.74; conventional-code geomean ≈ 1.10).

| Workload | LCCC vs GCC | Root cause | Oracle (CE -O2 v3) |
|----------|-------------|------------|---------------------|
| gzip `longest_match` | 118 vs 0 stack-mem, 248 B frame, GOT | RA remat + RIP + IV homes (RA-01/02) | gcc `window(%r9,%rcx)`, 1× push rbx |
| zlib-ng Adler kernel | ~1.50× | arithmetic-chain copy webs + no reload-at-use (RA-06/RA-03) | icx/clang: 0 stack |
| gzip CRC kernel | **0.87× WIN** (hoisted PIC base + magic-const hoist) | closed for the table loop | gcc 20-line loop; **no `crc32` insn** |
| Expat name scan | ~1.80× | hash-multiply `imul` chains; branch-prediction shaped on VM | gcc `btq` |
| xmltok / inflate TUs | segment RA landed; re-measure | former 12×/15× stack-mem addressed by RA-05a | — |
| `struct_copy` | ~1.35× | scalar movsd chains vs paired mulpd; SysV SSE class (AB-01/02) | gcc/icx: 2× ymm for 64 B |
| nbody / spectral / mandelbrot | 1.23 / 1.31 / 1.23 | multi-store scatter (OP-05b) + marching pointers (RA-01b) | ICX nbody 97 ymm / 58 FMA |
| sqlite varint | 1.19× | ISel branches, loop rotation | — |
| linux_find_bit | 1.51× | gcc `andn`+`cmov` ffs tree, **not tzcnt** | CE corpus |
| loop_patterns | 1.06× (parity within CI) | LCG RA precise-span seed + fused mul-add | gcc `vpmaxsd` would beat scalar |
| sieve | 1.20× | gcc **45 scalar**; clang **365 ymm** — **do not copy clang** | |
| mandelbrot | 1.23× | FP branch-heavy loop; rotation | |
| libm_round_family | **0.41× (2.42× faster)** | IS-29a landed | |
| fib / ackermann | **0.010× / 0.018×** | TCE + rec2iter | not codec metrics |

## 4. What "beat ICX" means

- **Integer codecs:** remat+RA+SIB (match, Adler, CRC) then ICX XMM-spill if
  still behind.
- **FP:** copy **ICX** `vfmadd231pd` YMM accumulator, **not** GCC's
  per-iteration horizontal add.
- **Aggregates:** SysV register class + ymm copies.
- **Parsers:** `btq`/range, segment RA, do **not** mega-inline (the varint
  experiment failed).
- **Bit kernels:** gcc `andn`+`cmov` on find_bit; `popcntl` on bitops; do
  not copy clang sieve ymm.

## 5. Landed register (do not re-fix; details in git history + DECISIONS.md)

PF-00..04, PF-08..14 (peephole/RA ledger v1) · PF-07 PIC table-base hoist ·
P0-01a/b, P0-03..05 · RA-04, RA-05(a), RA-12, RA-13, RA-21, RA-23..27 ·
IS-01, IS-03, IS-04, IS-06, IS-12, IS-14, IS-15, IS-20, IS-24, IS-28,
IS-29, IS-30(super) · OP-01, OP-04, OP-06, OP-12, OP-13(same-block),
OP-15, OP-31, OP-34 · OP-05a (stencil + map trees) · UN-01 (general
complete unrolling) · FE-01, FE-21, FE-22 · AB-09, AB-13, AB-14,
AB-06/07 (session 90) · PG-03 · MS-01a (gate wired), MS-02, MS-07 ·
LK-04 (CASP/MOVW/.org/PREL64), LK-17 (pcre2), LK-26 (seg_fs lvalue stores,
`afa22485`) · loop rotation infrastructure v13–v17 (opt-in) · DSE
same-block · backedge PRE · GVN disjoint epochs · FpContract tri-state ·
mcount/`-pg` family · 32 KiB boot gate PASS · i686 natural slots ·
width-partitioned slots · RISC-V va_arg struct{long double} · ci-codegen
gate. · aarch64 f128-carrier full-precision plumbing (U128 loads/stores feed
the f128 tracker; pair loads prefer it; x9 dead-address-mov full-scan
soundness; f128_softfloat + f128_global_carrier green, 2026-08-28). · riscv f128-carrier
end-to-end (f128_in_gp_pairs classify arm, tracker-fed U128 load/store,
sign-bit intrinsic arms — F128Neg previously hit the silent SIMD no-op;
f128_softfloat + f128_global_carrier green on qemu-riscv64, 2026-08-28). ·
AAPCS64/RISC-V wide va_arg: __int128 va_arg arms both backends, long double
varargs full precision on riscv, align-16 composites at even pairs (named +
variadic, both arches — old "AAPCS64 needs no even pairs" note was wrong),
va_arg_wide_struct green on aarch64 AND riscv64 (2026-08-28).

Rejected-with-evidence (never resubmit without new data): Agent B's
MAX_SMALL_INLINE_BLOCKS 12 / -Os cap removal / split-call-everywhere;
zero-future-use eviction; adler accumulator-leader promotion;
`LCCC_BEST_EFFORT_NO_HOME`; `scalar_storage_order` acceptance; call-spanning
F64 → callee-saved XMM; SSO bitfield rework; within-32-bit Cast hazard
relaxation; `CCC_CALLER_SAVE_SPANNING=1` on i686. See DECISIONS.md.
