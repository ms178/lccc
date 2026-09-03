# Session 30b — folded-address liveness, `load_reuse` self-address loads, and a measured `reassoc_accum` pessimization

Base: `ms178/lccc` `77eb34e` (branch `work`). Date: 2026-09-03.

## 0. Executive summary

| # | Defect | Symptom | Status |
|---|--------|---------|--------|
| A | Chained-GEP folded-address liveness (`extend_gep_base_liveness`) | sqlite 3.50 `-O1` **SIGSEGV** (the open bug since session 11) | **FIXED** |
| B | `reuse_redundant_loads` peephole: load whose destination is one of its own address registers | wrong pointer returned from an inlined hash lookup (`-O2`) | **FIXED** |
| C | `reassoc_accum` applied unconditionally | **1.7x–2x slowdown** and +30…80 % code size on its own flagship pattern | **FIXED** (cost model) |

Headline numbers:

```
sqlite 3.50.4 (mini2 oracle, -O1)      before: SIGSEGV after open=0   after: open=0 create=0 rc=0
k01_adler adler8, Godbolt instruction  before: 103                    after:  70
                                       gcc 16.2: 129, clang 23.1: 75, icc: 68, icx: 65
adler32_do8 (repo bench, -O2)          before: 5.41 ms                after: 4.79 ms   (gcc 4.00 ms)
sqlite3.c -O2 code size                +0.15 % (.text), +0.12 % insns  (cost of keeping roots live)
```

Battery after all three changes: regression **578 PASS / 0 FAIL / 15 SKIP** (AB-diff 0),
`cargo test --lib` **1664 passed / 0 failed**, slot stress **144/144**,
gep-chain stress **600/600** (the same 600 cases contain **11 failures** on the unfixed compiler).

---

## 1. Defect A — chained-GEP folded-address liveness (sqlite `-O1` SIGSEGV)

### 1.1 Symptom and path to the root cause

Sessions 11–12 established: `CCC_DISABLE_PASSES=inline` makes sqlite `-O1` correct,
`CCC_VERIFY_IR` and `CCC_INLINE_VALIDATE` are both silent (0 lines), and gdb reports

```
SIGSEGV at sqlite3FindIndex+368   rbp=0x1   frames: #0 sqlite3FindIndex, #1-#3 ??,
                                  #4 sqlite3FindTable, #5 rip=0x0
```

The callee-set bisection did not converge (halfA PASS / halfB SEGV / q1 PASS / q2 PASS),
so this session went straight to the emitted code instead.

`rbp = 0x1` is not a frame-pointer mystery: `rbp` was **the loop counter `i`**. The
failing instruction is

```
sqlite3FindIndex+0x170:  mov 0x8(%r10),%rax     ; r10 = ht + (h % htsize)*16
   rsi(ht) = 0xb800000000005706   rbx = 0x5702f9   r13(zName) = 0x5702f8 -> "t"
```

`rbx` is `zName+1` — the *post-hash-loop* value of the `strHash` walker. `rbx` should
have been `pSchema`. The inlined `strHash` loop simply took the register that still
held `pSchema`.

### 1.2 The mechanism

The IR (post-inline) is:

```
b5:  36 = Load [&aDb[j].pSchema]              ; pSchema            (root)
b12: 47 = GEP(36, +32)                        ; &pSchema->idxHash  (intermediate)
b7:  145 = GEP(47, +16)   ; Load [145]        ; idxHash.ht
b8:  151 = Load [47]                          ; idxHash.htsize
b16: 147 = GEP(GEP(145'), +8) ...
b17: 153 = GEP(47, +8) 156 = GEP(47, +4)      ; idxHash.first / .count
```

The backend composes const chains (`compose_const_gep_folds`), so every one of these
accesses is emitted as `disp(%pSchema)` — e.g. `mov 0x30(%rbx),%r10` for
`pSchema+(32+16)`. The register the access reads is the **root**, `v36`.

`CCC_DEBUG_LIVE=36` on the unfixed compiler:

```
[LIVE] v36 def=28 last_use=129
  block 12 (label 28734) live_in=true live_out=false pts=[55,58]     <- the GEP
  block 27 (label 28749) live_in=true live_out=true  pts=[126,127]
  block 28 (label 28750) live_in=true live_out=true  pts=[128,129]
  segments=[(28, 52), (55, 55), (126, 129)]
```

The folded accesses live at points **73…95** — inside the *hole* `56…125`. The hole
exists because the caller's `continue` blocks (b27/b28) sit on the def→use path but
are laid out **after** the inlined body. Per IR liveness `v36` really does die at its
GEP (point 55), so recycling the register is legal — the liveness model, not the
allocator, was wrong.

Two independent causes inside `extend_gep_base_liveness` (`src/backend/liveness.rs`):

1. **`non_foldable` dropped every GEP dest that feeds another GEP.** `v47` is the base
   of `v145/v153/v156`, so the `_ =>` arm marked it non-foldable and removed it from
   `gep_info`; the `Load { ptr: v47 }` (htsize, offset 0) therefore never extended `v36`.
2. **Only the *immediate* base was extended.** For `Load { ptr: v130 }` with
   `v130 = GEP(v47, +16)`, the code extended `v47`, never walking up to the root `v36`.

### 1.3 Fix

`extend_gep_base_liveness` now

* classifies every use of every candidate GEP dest into **hard non-fold** (call
  argument, stored value, Cmp operand, terminator operand, i128 access, …) and
  **address link** (base / variable offset of another GEP);
* computes foldability as a **fixed point** over the address-link graph (a dest whose
  consumer was dropped is materialised, so its own base becomes a real use);
* walks the **whole composed chain** at every folded Load/Store and extends every link
  — root, intermediates and the variable index — to the access point, stopping at the
  first link the backend cannot fold into one addressing mode (second variable index,
  displacement outside `i32`), which is exactly where the backend materialises an
  address of its own;
* keeps the `folded_bases` ranking set the allocator uses so a hot base with one
  recorded use does not lose its register.

`CCC_GEP_CHAIN_LIVENESS_DEPTH=1` restores the pre-session-30 immediate-base-only walk
for A/B measurement.

### 1.4 Evidence

```
sqlite3.c -O1 + mini2 oracle     before: SIGSEGV (rc 139)      after: open=0 create=0 (rc 0)
tests/regression/gep_chain_fold_root_liveness.c (-O1)
        fixed compiler: PASS     CCC_GEP_CHAIN_LIVENESS_DEPTH=1: SIGSEGV
scripts/run_gep_chain_stress.sh 1 60 -O1 -O2 -Os
        fixed: 180/180           chain walk disabled: 7 failures (all -O1: seeds 9,12,14,18,20,27,40)
```

---

## 2. Defect B — `load_reuse` and loads that overwrite their own address register

Found by the new stress harness (seed 12, `-O2`) while validating defect A.

Raw codegen vs. peephole output for a hash-bucket walk:

```
raw                              peephole
leaq 8(%r11), %r15   # &b->chain leaq 8(%r11), %r15
movq (%r15), %r15    # chain     movq (%r15), %r15
movq (%r15), %r12    # chain[0]  <- deleted, replaced by "movq %r15,%r12" then coalesced
```

`dead_writes::reuse_redundant_loads` rewrites a later load to a copy of the earlier
load's destination when both have the same opcode and the same *textual* memory
operand. It validated every line **between** the two loads (address-register writes,
memory writes, calls) but not the first load **itself**: `movq (%r15),%r15` overwrites
its own address register, so the second `(%r15)` reads a different location — the value
just loaded, not `chain[0]`. The lookup returned the `char **` instead of the string.

**Fix:** refuse the substitution when the first load's destination family is also one
of its own address-register families (`addr_fams.contains(&dst_fam)`).

**Evidence:** `tests/regression/peephole_load_reuse_self_addr.c` (`-O2`) passes on the
fixed compiler and fails on the unfixed one; the unfixed compiler fails 11/600
gep-chain stress cases at `-O2`/`-Os` (seeds 18, 20, 40, 74, 137, 141, 156, 171, 190).

---

## 3. Defect C — `reassoc_accum` was a pessimization

The Godbolt oracle flagged the Adler-32 kernel first:

```
k01_adler.c, function adler8 (instructions)
  lccc (before) 103   gcc 16.2 129   clang 23.1.0 75   clang trunk 75   icc 68   icx 65
```

Bisecting `-O2` with `CCC_DISABLE_PASSES` isolated **`reassoc_accum`** as the only pass
that moved the number (105 → 70 when disabled). Direct A/B over the source-unrolled
zlib DO8 loop (1 MiB × 30 reps, `-O2`):

| unroll N | insns on / off | runtime on / off |
|---:|---:|---:|
| 4  | 75 / 58  | 21.7 / 10.2 ms |
| 6  | 93 / 64  | 19.1 / 11.5 ms |
| 8  | 106 / 70 | 18.7 / 11.1 ms |
| 12 | 134 / 82 | 17.1 / 10.7 ms |
| 16 | 169 / 94 | 20.3 / 10.5 ms |

The pass replaces `N` dependent adds with `2N+2` mostly-independent ops *and* keeps all
`N` byte terms plus all `N` weighted terms live to the splice point. It only wins when
the **loop-carried recurrence**, not issue throughput, is the binding constraint — and
both accumulator phis advance by a single 1-cycle add per iteration, so an
out-of-order core overlaps iterations freely. Since the pass requires `N >= 4`, its
target loops always contain at least 4 loads + 8 adds and are throughput-bound: the
transform cannot pay off on any superscalar target. The non-unrolled `k_adler32.c`
never triggers the pass at all, which is why the repository benchmark never saw this.

**Fix:** a real cost model — estimated cycles `max(recurrence, ops/issue_width)` before
and after, plus a peak-register-pressure bound (`2N+4 <= usable GPRs`). The transform is
retained (not deleted) and stays reachable for targets where the trade-off inverts via
`CCC_REASSOC_ACCUM_FORCE=1`.

**Evidence:**

```
k01_adler adler8 instructions   105 -> 70     (beats gcc 129, clang 75, icc 68; icx 65)
scripts/bench_kernels.py --kernels adler32_do8 --flags=-O2
        cost model 4.788 ms   forced 5.406 ms   gcc 4.0 ms
```

---

## 4. New test infrastructure

| file | purpose |
|---|---|
| `scripts/gen_gep_chain_stress.py` | generates deterministic C programs stressing folded-address liveness |
| `scripts/run_gep_chain_stress.sh` | differential runner (lccc vs gcc) over seed ranges × opt levels |
| `tests/regression/gep_chain_fold_root_liveness.c` (+`.flags` `-O1`) | frozen generator seed 12 — kills defect A |
| `tests/regression/peephole_load_reuse_self_addr.c` (+`.flags` `-O2`) | frozen generator seed 74 — kills defect B |
| `tests/bench/k_adler32_do8.c` | the source-unrolled DO8 loop `reassoc_accum` targets (measurable in-repo) |

Generator axes (all seed-driven): GEP chain depth 2/3, byte offsets, five loop shapes
(hash, hash+length, hash+two accumulators, indexed scan, two-walker chase), folded
reads before/after the loop, caller-side `continue` arm (which creates the live-range
hole), 0…8 values live across the call, 2–3 databases, 8/16 buckets, 3–4 probe keys.
Every program is strictly well-defined C, so any divergence from GCC is a miscompile.

```
scripts/run_gep_chain_stress.sh 1 200 -O1 -O2 -Os
   unfixed compiler: 11 failures (seeds 18,20,40,74,137,141,156,171,190 at -O2/-Os)
   fixed compiler:   600/600 PASS
```

---

## 5. Code-size cost of the liveness fix (sqlite3.c, 250 kLOC)

| config | `.text` bytes | instructions |
|---|---:|---:|
| before `-O1` | 1 408 059 | 400 113 |
| after  `-O1` | 1 410 939 (+0.20 %) | 400 849 (+0.18 %) |
| before `-O2` | 1 448 331 | 409 538 |
| after  `-O2` | 1 450 459 (+0.15 %) | 410 034 (+0.12 %) |

Keeping every chain link live to every folded access costs ~0.15 % of code size on the
largest available corpus. Intermediates are extended as well as roots because a chain
can be *partly* materialised (one `lea`, one folded displacement), in which case the
intermediate register is genuinely read at the access.

---

## 6. Battery (all with the three fixes applied)

```
scripts/run_regression_suite.sh         PASS=578 FAIL=0 SKIP=15 (AB-diff 0)
scripts/run_slot_stress.sh 1 12 -O1 -O2 -Os   PASS=144 FAIL=0
scripts/run_gep_chain_stress.sh 1 200 -O1 -O2 -Os  PASS=600 FAIL=0
cargo test --lib                        1664 passed, 0 failed, 6 ignored
sqlite 3.50 mini2 oracle -O1/-O2        correct
```

---

## 7. Follow-ups (deferred, with the data needed to continue)

1. **k01_adler: 70 vs icx 65.** ICX wins by 5 instructions; the remaining gap is
   backend scheduling (lccc still re-loads a byte into a fresh register per step).
   Re-measure with `scripts/godbolt.py compare` after any scheduling change.
2. **`-O1` vs `-O2` on adler8**: 75 vs 70 instructions — the `-O1` pipeline leaves
   something on the table that `-O2` recovers elsewhere. Worth a pass-level diff.
3. **MachInst exhaustive stress**: the harness added here exercises the *folded-address*
   contract; an equivalent generator for the MachInst layer (wide immediates, typed
   indirect calls, memory relays from PR #363/#364) is still missing.
4. **Kernel build/boot** (`linux-cachymod-6.18.47` under QEMU) — untouched this
   session; `KERNEL_DIR` is not set, so the regression suite's boot gate skips.
5. **More Godbolt oracles**: only `k01_adler` was measured this session. The corpus in
   `tests/benchmark/kernel_corpus/` (15 kernels) should be swept against
   `cg162`, `cclang2310`, `cclang_trunk`, `icx`, `icc` and the deltas triaged.

## 8. Appendix — reproduction

```bash
# defect A / B differential harness
scripts/run_gep_chain_stress.sh 1 200 -O1 -O2 -Os
LCCC_BIN=<unfixed-lccc> scripts/run_gep_chain_stress.sh 1 200 -O1 -O2 -Os

# defect A liveness dump on sqlite3.c
CCC_DEBUG_LIVE=36 lccc -O1 -S sqlite3.c -o /dev/null
CCC_DUMP_IR=1 CCC_DUMP_IR_FUNC=sqlite3FindIndex lccc -O1 -S sqlite3.c -o /dev/null

# defect C
scripts/bench_kernels.py --kernels adler32_do8 --flags=-O2
CCC_REASSOC_ACCUM_FORCE=1 scripts/bench_kernels.py --kernels adler32_do8 --flags=-O2
python3 scripts/godbolt.py compare tests/benchmark/kernel_corpus/k01_adler.c \
    --local target/fastbuild/lccc --oracles cg162,cclang2310,icx,icc --function adler8
```
