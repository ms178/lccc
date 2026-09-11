# Follow-up: Astra `regalloc.rs` kernel + next-batch P0s (v2)

Date: 2026-09-10 · Base: `ms178/lccc` main `@ 28379879` (PR #483)

## 1. Scope

Proven P0–P3 from Astra V1/V2 plus remaining semantic blockers closed
with in-file contracts. No second replacement of the same function.
SIMD *allocator* rewrite and accumulator-on-non-divrem stay deferred.

`cargo test --profile fastbuild --locked -j2 --lib backend::regalloc`: **54 passed**.
`./scripts/ci_local.sh --fast` with `LCCC_GAS=$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin/as`:
**16 passed, 0 failed, 3 skipped — ALL GATES GREEN**.

## 2. This batch vs Astra

Astra's remaining P0 was “delete `phi_web_class`”. That is too coarse:
exclusive CFG arms of one C variable *should* share a home; simultaneously
live segments must not. The landed rule is **hole-aware**:

```
same phi-web AND NOT sorted_coverage_overlaps(segments(a), segments(b))
```

Missing segments fall back to the fat envelope (fail-closed). Latch
`same_value_edges` and no-op casts keep their local proofs.

`source_home_survives_dest_redefs` is an instruction-level dirty-state CFG
walk (selected Copy exempt). Extra `src` defs **refresh** `dirty=false`
(phi-elim reuses ids across latches). Folded-index `hidden` reads are
**not** consulted here: liveness attributes `GEP(buf,1)` loads to the
later `buf+8` id, which is not an IR-visible use of the increment.
Dest folded reads in the update window stay in `detect_phi_coalesce`
(`phi_live_in_window`).

Vecreg: unknown `dest_ptr` writers poison the slot; only 16-byte allocas
promote. Synthetic slot liveness is CFG dataflow, not layout-adjacent
loop-depth runs. FP phi rehoming refuses call-spanning sources on
caller-saved XMM/volatile NEON.

## 3. Adjudication (full)

| Finding | Verdict | Action |
|---|---|---|
| `phi_web_class` fat-interval exemption | **Agree, not delete** | Segment overlap is the proof |
| Source-path skips copy-block redefs | **Agree** | Dirty-state instruction walk |
| Unique-src-def abort | **Disagree** | Refresh; phi-elim reuses ids |
| Folded `src` reads on dest-base loads | **Disagree** | IR-visible uses only |
| Unknown vecreg `dest_ptr` writers | **Agree** | Poison `bad_use` |
| 16-byte slot vs 32-byte alloca | **Agree** | `size != 16` ineligible |
| `synthetic_vec_intervals` layout heuristic | **Agree** | CFG live-in/live-out |
| FP-phi rehome ignores calls | **Agree** | `spans_any_call` + callee-FP |
| `hottest_latch` silent if `<2` candidates | **Agree** | Hard assert |
| `args[1]` in SSE chain collector | **Agree** | `args.get(1)` |
| Accumulator-on-non-divrem | **Hold** | RISC-V/AArch64 `stack_layout` |
| SIMD SSA allocator rewrite | **Defer** | Producer/consumer/clobber matrix |
| Late i686 hazard vs eviction | **Open** | Needs emitter commitment |
| Mul-acc SSA stamps | **Open** | Next |
| `FunctionRaFacts` / cost models | **Later** | After legality |

## 4. Validation

```
cargo test --profile fastbuild --locked -j2 --lib backend::regalloc
# 54 passed

LCCC_GAS=$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin/as
./scripts/ci_local.sh --fast
# 16 passed, 0 failed, 3 skipped — ALL GATES GREEN
# (GAS 2.47.20260726; rustc 1.98.1; i686-asm-diff 20/20)
```

### zlib_ng_adler32 (codegen-quality-gate)

Baseline: `insns=233 stackmem=11 pushes=6 moves=60`.

| Mode | insns | stackmem | pushes | moves |
|---|---|---|---|---|
| This patch (default) | **230** | **11** | 6 | **57** |
| `CCC_NO_PHI_COALESCE=1` (pre-fix) | 244 | 11 | 6 | 71 |
| False-reject source_home (pre-fix) | 233 | **12** | 6 | 59 |

Inner-loop `buf += 8` (`v727 = v703`) and the sibling latch `v725/v529`
now coalesce. Gate stackmem stays 11; insns −3, moves −3 vs baseline.

A/B census (`ra_ab_census.py --env CCC_NO_PHI_COALESCE=1` on the same
file, driver included): default vs no-coalesce `insns 246→260`,
`rrmov 47→61`. Coalesce is a net win on the quantities the gate tracks.

### Kernel corpus vs GCC 14.2 `-O2`

`scripts/ra_quality_census.py --kernels --no-clang`: LCCC **266** / GCC **264**
insns across 15 hot functions (LCCC fewer on 5, equal 3, more 7). Unchanged
vs the prior kernel snapshot. Clang 23.1 / ICX / GCC 16.2 are not on this box.

GAS 2.47 is the asmdiff oracle (`LCCC_GAS`). Patch vs `28379879`:
`/home/user/ms178-1.patch`.
