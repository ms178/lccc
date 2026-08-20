# Unified next-agent handoff (`ef6511e`)

**Read in this order:**

1. This file (operating rules + CE corpus deltas + extra bugs).
2. [`AGENT_BRIEFING_150.md`](AGENT_BRIEFING_150.md) — 150 ROI items, architecture flaws, first-10 sequence.
3. [`GODBOLT_CORPUS_SCOREBOARD.md`](GODBOLT_CORPUS_SCOREBOARD.md) — 28 kernels × gcc16.2/clang22/icx.
4. `lccc-improvements/register-allocation/VALIDATION_ZLIB_GZIP_EXPAT.md` — LCCC vs GCC14 `-S`/runtime.
5. `hotspots/godbolt-corpus/*.s` — raw CE asm.

**Tree:** `ms178/lccc` `ef6511e`. **Do not** implement from March 2026 RA Week-2 docs.

---

## Operating rules (non-negotiable)

| # | Rule |
|---|------|
| 1 | Correctness: `cargo test --lib` + relevant fuzz; SROA/RA/MachInst need sqlite/expat/gzip if touched. |
| 2 | **gzip `longest_match` stack-mem is the RA veto.** |
| 3 | Re-oracle after every patch: `scripts/godbolt.py compile gcc16.2 tests/benchmark/programs/KERNEL.c --flags '-O2 -march=x86-64-v3'` |
| 4 | **Copy ICX FMA**, not GCC 16.2 per-iter horizontal (`dot`/`reduction`/`spectral`). |
| 5 | Do not enable MachInst on large loops (`CCC_MI_MAX_LOOP_INSTS`, gzip −3%). |
| 6 | Do not `CCC_SROA_COPYOUT` without dominators. |
| 7 | Do not `CCC_EVICT_MODE=5` or `CCC_PGO_WEIGHT_MAX>1`. |
| 8 | PhysReg(11)=`%r10`, PhysReg(10)=`%r11`. |
| 9 | Never cite ICX CRC on a **zero** table. This corpus uses the real gzip table. |
| 10 | Fib/TCE geomean is not a codec win. |

---

## LCCC vs oracles — what is uniquely broken

| Gap | LCCC (measured) | Oracle (`-O2 -march=x86-64-v3` CE, this run) | Ticket |
|-----|-----------------|-----------------------------------------------|--------|
| gzip `longest_match` | 118 stack-mem, GOT, 248 B | gcc match-mini: RIP `window(%r9,%rcx)`, 1 push | RA-01/02 |
| Adler DO8 | `sum2`/`n` on `%rsp` every 8 B, 1.49× | **whole-file** gcc stk=2 clang=0 icx=3 | RA-03 |
| CRC | 1.47×, 2 vs 0 spills | gcc `xorl gzip_crc32_table(,%rdx,4)` **no `crc32` insn** | IS-01 |
| find_bit | 1.85× S | gcc **`andn`** + **cmov** on ffs tree (**not tzcnt**) | IS-ANDN, IS-CMOV-FFS |
| bitops popcount | ? | gcc/clang **`popcntl`** | IS-POPCNT (alu.rs already has it — check IR) |
| expat classify | 1.95× | gcc **`btq`**; icx ymm on fill | IS-06 |
| nbody | 2.9–3.8× S | gcc **24 FMA scalar**; icx **58 FMA + 97 ymm** | OP-05, IS-04 |
| matmul | ~1.1× S | gcc 1 FMA ymm; **icx 8 / clang 16 FMA** | IS-04 |
| spectral | 9.3× S | gcc ymm **103 / 0 FMA**; icx FMA **10** | IS-04 |
| reduction | | gcc ymm no FMA; **icx/clang FMA 8** | IS-04 |
| mandelbrot | 3.2× S | all scalar FMA; icx 7 vs gcc 3 | IS-04 |
| struct_copy | **21.06×** | gcc/icx FMA on **distance**; not 2× ymm of Particle | AB-01, IS-02 |
| sieve | 1.3× S | gcc **45 scalar**; clang/icx **365 ymm fill** | **do not copy clang** |
| arith_loop | 1.10× S | gcc **107 stk**; clang 83 ymm integer | RA pressure |
| varint | ~2× S | 325–421 ins, no magic ISel | don’t mega-inline |

LCCC already implements: scalar+vector FMA **emitters**, ymm **memcpy**, `lzcnt`/`tzcnt`/`popcnt` **UnaryOp**, SROA 1–2, `alias.rs`, MachInst (gated), load-cast fold, cmp fusion.

**Shortcomings are idiom recognition, RA homes, ABI, and vectorizer breadth — not “missing popcnt instruction.”**

---

## Extra tickets from full-corpus CE (add to the 150)

| ID | ROI | Item | Evidence | Accept |
|----|-----|------|----------|--------|
| IS-ANDN | P0 | Lower `a & ~b` / `~a & b` to `andn` | find_bit gcc 3× `andn`; icx `andnq` | CE find_bit uses andn |
| IS-CMOV-FFS | P1 | Generic `__ffs` if-tree → `tzcnt` **or** gcc’s `cmov` chain | gcc cmov×11 not tzcnt | beat 1.85×; don’t assume tzcnt |
| IS-POPCNT | P0 | `popcount32` C loop → `UnaryOp::Popcount` / `popcntl` | bitops gcc `popcntl`; alu.rs:94 already emits | bitops uses insn |
| IS-CLZ-LOOP | P1 | `clz32` C loop → `lzcntl` | alu.rs has lzcnt | bitops |
| OP-NO-SIEVE-VEC | P0 | Do **not** vectorize sieve fill like clang (365 ins) | gcc 45 ins scalar wins simplicity | no size explosion |
| OP-NBODY-YMM | P0 | nbody: ICX 97 ymm vs gcc 0 — **ICX is the bar** | CE | ymm + FMA |
| OP-SPECTRAL-FMA | P0 | spectral gcc already ymm; gap is **FMA 0 vs icx 10** | CE | vfmadd in vector body |
| OP-STENCIL-MEM | P1 | stencil5 all three ymm; LCCC fp_memfold path | CE ymm 36–45 | keep memfold |
| IS-STRLEN | P2 | ICX 40 calls — ignore; gcc 129 | noisy | |
| RA-ARITH-STK | P1 | arith_loop gcc 107 stk vs clang ymm | 32-live RA | ≤ gcc stk |
| IS-HASH | P2 | hash_table ~116–142, 6 gcc calls | | |
| OP-VARINT | P1 | sqlite_get_varint branch cluster; 189-inst inline **rejected** | briefing | ISel not inline |

---

## Bugs / inefficiencies / structural (expanded)

### Codegen / RA

- Fat intervals vs `segments` (xmltok/inflate).
- `enable_splitting` dead.
- Dual RA (IR + MachInst).
- Dual coalescers.
- Location = HashSet pile (`never_materialized`, `fused_cmp_*`, `cmp_replay`, `load_cast_fold`, nohome classes).
- `cmp` excluded from nohome (replay vs %rax).
- GOTPCREL for file-scope (`globals.rs`) vs gcc RIP.
- FP field `movq %xmm,%rax` (`memory.rs:1042` comment).
- `vectorize_gate` always true.
- LICM GEP TODO despite `alias.rs`.
- SROA copy-out off (hangs).
- MachInst scheduler gzip −3%.
- Text peephole vs MachInst.
- PhysReg r10/r11 swap.
- `exclude_every_third_mul_temp` magic.
- OnceLock env knobs (tests can’t vary mode).

### Frontend / IR

- Sema ~1 diag vs lowerer ~382.
- No TBAA/`restrict` on IR.
- ExprId = pointer.
- Preprocessor is text.

### Process

- Validation `-S` was pre-#148 — **re-run longest_match on this SHA first**.
- No LCCC binary in this CE run — next agent must `build_lccc_fast.sh` and `--local` compare.

---

## First 10 (updated with CE)

1. **MS-07** LCCC `-S` `longest_match` stack-mem on `ef6511e`.
2. RA explain dump.
3. RIP-relative `window` (CE gzip not applicable; use deflate.c / match-mini).
4. Adler next-use (CE: oracles ~0 stack).
5. CRC SIB (CE: gcc table `,%reg,4`).
6. **`andn` + popcnt IR** (CE find_bit/bitops).
7. XMM field / SysV SSE (struct_copy 21×).
8. ICX-style **YMM FMA acc** on nbody/matmul/spectral/reduction (CE FMA counts).
9. `btq` classify (CE expat gcc).
10. LICM ← `alias.rs`.

---

## Reproduce CE corpus

```bash
for f in tests/benchmark/programs/*.c; do
  b=$(basename "$f" .c)
  python3 scripts/godbolt.py compile gcc16.2 "$f" --flags '-O2 -march=x86-64-v3' \
    > hotspots/godbolt-corpus/${b}_gcc16.2.s
done
```
