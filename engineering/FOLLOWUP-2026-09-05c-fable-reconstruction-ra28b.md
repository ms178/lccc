# Follow-up work — 2026-09-05 (session 3) — Fable reconstruction, RA-28b/RA-06 specified

Base: `ms178/lccc` main @ `9184d66` (contains PR #412 and PR #413, i.e. both
previous sessions' work).
Deliverable: `ms178-1.patch`, verified `APPLIES-CLEAN` on that commit.

---

## 1. Accomplished

### 1.1 Two live miscompiles found and fixed (Fable reconstruction)

Both were reconstructed from the fragmentary diff, **re-derived from first
principles, and empirically confirmed to still reproduce on this base** before
being fixed — neither was taken on faith.

**CG-07 — `-0.0` in static data collapsed to `.zero`.** The array emitter
built `.zero N` runs with `IrConst::is_zero()`, which is C *truthiness*
(`-0.0 == 0.0` is true). Measured before the fix:

```
static const float t[4] = {-0.0f, 0.0f, -0.0f, 1.0f};
t[0] bits=00000000 signbit=0        // want 80000000 / 1
1/t[0]  ->  +inf                    // want -inf
```

New `IrConst::is_all_zero_bits()` is the object-representation predicate
(`F32`/`F64` by `to_bits()`, `LongDouble` by its byte array, `D32`/`D64` by
their BID container, integers delegate since they have one zero encoding).
The NULL-pointer test in `const_eval_global_addr.rs` deliberately keeps
`is_zero()` — truthiness is the right question there.

**VEC-12 — a map loop dropped every loop-carried value except the counter.**
The packed body runs one iteration per W elements and reconstructs no scalar
recurrence, so a second carried value ends the loop holding its value after
`n/W` steps. Measured at `-O2`/`-O3` with an 8-lane body:

```
for (i) { d[i] = a[i]*2; acc += 3; }  return acc;   ->  0 at n = 1  (want 3)
for (i) { d[i] = *p++ * 2; }          return p - a; ->  n/8
```

The IR dump showed the packed loop faithfully carrying the `acc` phi while
stepping the counter by 8 — the recurrence was kept and its trip count was
not. Fixed fail-closed in the map analyzer.

**A finding the source diff did not contain:** rejecting the shape in the map
analyzer alone did **not** fix the bug. The rejected loop simply fell through
to `analyze_stencil_pattern`, which miscompiled it identically. The guard had
to go into both analyzers. This is only visible if you re-run the reproducer
after the first fix, which is why the fix was verified rather than assumed.

### 1.2 The fourth strict min/max fold (VEC-11)

Upstream folded three of the four strict ternary forms; `l < r ? r : l` fell
through to compare+blend. The exactness argument is **operand order**, and it
is worth stating precisely because the previous comment concluded the opposite:

> Intel's packed MIN/MAX are, lane for lane,
> `MIN(src1,src2) = (src1 < src2) ? src1 : src2` and
> `MAX(src1,src2) = (src1 > src2) ? src1 : src2`, with **every** special case
> (either operand NaN, both operands zero of any sign) returning **src2**.
> A strict-ordered ternary therefore folds exactly whenever its FALSE arm is
> placed in src2. Since `l < r` ≡ `r > l`, `l < r ? r : l` is
> `MAX(src1 = r, src2 = l)` — false arm `l` in src2. Exact.

The old comment rejected the form because `max(l, r)` is wrong on a ±0 lane —
true, but the fix is the operand order, not abstention. `MapExpr::MinMax{l, r}`
emits args `[l, r]` with `r` as src2, which is what makes it work.

Payoff: `x = x<0?0:x; d[i] = x>1?1:x;` now lowers to

```asm
vmovups (%rsi,%r10), %ymm4
vmaxps  %ymm4, %ymm3, %ymm5     ; clamp head  <- the new fold
vminps  %ymm5, %ymm2, %ymm6     ; clamp tail
vmovups %ymm6, (%rdi,%r10)
```

**GCC 14.2 does not vectorize that loop at all** (zero `vmaxps`/`vminps`).

### 1.3 Integer map lane ops (VEC-10)

`VecSub/And/Or/Xor I32x4|x8` and `VecSubI64x2`, wired through the intrinsic
table, the x86 emitters (`vpsubd`/`vpand`/`vpor`/`vpxor`, SSE2
`psubd`/`pand`/`por`/`pxor`/`psubq`), the map analyzer's admission gate and
the per-target op table. `Sub` is passed `commutative = false` so a folded
memory operand can only land in src2 (`vpsubd mem, %ymm_src1, %ymm_dst`).

Verified emitting on the corpus (`vpsubd` ×5, `vpand` ×3, `vpor` ×2,
`vpxor` ×3) and hash-identical to GCC at `-O0/-O2/-O3/-Os` and on i686.
`psubq` never fires — recorded as VEC-10's remaining item rather than left as
a silent dead lowering.

### 1.4 RA-28b answered — and it changes the RA-06 specification

RA-28b hypothesised that eviction mode 6 loses on hot kernels because of
*pure lifetime demotion*, and would flip sign once splitting existed. Tested
directly with `scripts/ra_ab_census.py`:

| experiment | kernel stkref | kernel insns |
|---|---|---|
| `CCC_EVICT_MODE=6` | +17 | +16 |
| `CCC_EVICT_MODE=6` + `CCC_SPLIT_MAX=200` | +17 | +16 |
| `CCC_SPLIT_MAX=200` alone | **0** | **0** |

Aggressive call-site splitting changes **nothing at all** on the corpus. The
reason is structural: the kernels that regress are **call-free leaf loops**,
so `split_ranges`'s call-split can never fire on them.

`arith_loop` is the canonical case — 32 simultaneously-live loop-carried
integers on a 14-register file, **zero calls**:

| | lccc | GCC 14.2 |
|---|---|---|
| stack refs in `arith_loop` | **165** | 92 |
| instructions | 270 | 240 |

So the hypothesis is confirmed in *mechanism* (permanent demotion is the
problem) but wrong about the *remedy*: the existing splitter is orthogonal to
RA-06, not a partial implementation of it. RA-06 needs **in-loop,
pressure-driven** splitting. The BACKLOG entry is rewritten accordingly, with
`arith_loop 165 -> <= 100` as the acceptance number and `ra_ab_census.py` as
the gate.

**Mode 6 remains opt-in.** The guard-rail is unchanged and still violated.

---

## 2. Validation

| gate | result |
|------|--------|
| `run_regression_suite.sh` | **632 PASS / 0 FAIL / 7 SKIP**, AB-diff 0 |
| `gcc.c-torture/execute` x86-64, `-O0..-Os` (8380) | 8368 pass, **0 run-fail** |
| `gcc.c-torture/execute` i686, `-O0..-Os` (8380) | 8355 pass, **0 run-fail** |
| new differential tests | integer lane ops and the clamp folds hash-identical to GCC at 4 opt levels on both targets |

Every remaining non-`pass` torture case is a **host-GCC** skip (GCC 14.2
cannot establish a reference), unchanged from the previous session: on
`pr118623`/`pr118915`/`pr119291` GCC aborts and lccc is correct; `pr117432`
needs C23 `va_start`, which lccc now supports and GCC 14.2 does not.

---

## 3. To-do

1. **RA-06** — now specified with an acceptance number; see the BACKLOG entry.
   The cost model (`remaining_cost`, per-use frequency) is already in place as
   the Belady input; what is missing is per-point pressure and the IR-level
   store/reload-as-fresh-SSA-name rewrite.
2. **VEC-12 follow-on** — re-admit loops with a secondary affine recurrence by
   *reconstructing* it in the epilogue (`acc0 + n*step`, `base + n*stride`)
   instead of refusing them. Currently a real vectorization opportunity is
   left on the table for correctness.
3. **VEC-10 follow-on** — make `psubq` reachable or remove the op.
4. **CG-07 follow-on** — audit the remaining emitter `is_zero()` call sites for
   the same truthiness/representation confusion.
5. RA-32, IC-05, FE-30, RA-29/30/31 as previously recorded.

---

## 4. Environment note (third wipe this project)

The harness wiped the worktree again, including `.git`, `~/.cargo`, and the
extracted torture corpus. The documented recovery worked exactly as written
and took about four minutes:

```
curl https://sh.rustup.rs | sh -s -- -y --default-toolchain 1.98.0 --profile minimal
git clone --depth 40 https://github.com/ms178/lccc          # work was already upstream
sudo apt-get install -y gcc-multilib libc6-dev-i386          # or i686 passes VACUOUSLY
bash scripts/ensure_gcc_torture.sh                           # /home/user/dl tarball survives
```

`/home/user/artifacts/*` and `/home/user/ms178-1.patch` survive; `lccc.bundle`
still does not restore (shallow source repo). Do not rely on it.
