# Follow-up: induction-variable widening fired only for byte arrays

Session date: 2026-09-01 (continuation)
Base: `ms178/lccc` main @ `11c8c988`
Snapshots: `S38`, `S39`

---

## 1. The finding

`iv_widen` exists to turn an `int` counter into an `i64` one so the backend
stops re-emitting `movslq` after every 32-bit `addl` — the narrow add clobbers
the upper half, so each address use needs a fresh sign-extension, and that
extension sits on the **loop-carried dependency path**. Removing exactly that
pattern from gzip's compare loop was worth 35% (S08).

A three-line probe shows it was almost never firing:

```c
int  sum_i8 (const signed char *a, int n){ int s=0; for (int i=0;i<n;i++) s+=a[i]; return s; }
int  sum_i32(const int  *a, int n){ int s=0; for (int i=0;i<n;i++) s+=a[i]; return s; }
long sum_i64(const long *a, int n){ long s=0; for (int i=0;i<n;i++) s+=a[i]; return s; }
```

| function | widened? | `movslq` in the loop |
|---|---|---|
| `sum_i8` | yes | 1 |
| `sum_i32` | **no** | 2 |
| `sum_i64` | **no** | 2 |

**Widening worked only for byte arrays.** The addressing analysis accepted
`Cast → GetElementPtr` but not `Cast → Shl(const) → GetElementPtr`, and the
`Shl` is exactly the element-size scaling every array wider than one byte
needs. So the single most common loop in C — indexing an `int` array — never
qualified.

## 2. The fix

`value_feeds_only_gep` now looks through a constant element scale, spelled
either as `Shl(index, k)` or `Mul(index, const)`.

Soundness is not a probabilistic argument: the scale is a loop-invariant
**constant** and the shift/multiply already executes at the wide type, so
widening the phi replaces the cast's destination with the `i64` phi and leaves
the scaling instruction untouched. A **variable** scale is deliberately *not*
transparent — the other operand could itself depend on the counter — and
`mul_by_variable_stride` in the regression test pins that.

## 3. Measurement

Paired **same-window** A/B with `CCC_NO_IV_WIDEN`, every output byte-identical:

| kernel | widened | not widened | ratio |
|---|---|---|---|
| **`sieve`** | 41.6 ms | 53.0 ms | **0.784** (−21.6%) |
| `nbody` | 386.7 | 398.5 | 0.970 |
| `arith_loop` | 141.8 | 143.6 | 0.987 |
| `sqlite_varint` | 30.2 | 30.6 | 0.987 |
| `aarch64_select_patterns` | 164.3 | 165.1 | 0.995 |
| `struct_copy`, `zlib_ng_adler32`, `fannkuch`, `tls_seg_access`, `spectral_norm` | — | — | neutral |

Widening additionally **unblocks vectorization** on `int`/`long` reductions
that previously stayed scalar, which is where most of the `sieve` win comes
from.

### 3.1 Why the corpus table does not show −21.6%

The 33-kernel run puts `sieve` at 1.043 against GCC, close to the previous
report's 1.034 — because **GCC's own time also moved** between runs on this
shared VM (41.0 → 36.6 ms). That is the cross-report trap from the last cycle,
and it is why the attribution above comes from a one-window A/B and the corpus
table is presented only as the current position. Both statements are true and
they measure different things.

## 4. What was measured and *not* pursued

- **Vectorization cost model.** GCC emits no SIMD at `-O2` on
  `spectral_norm`/`nbody`/`struct_copy`, lccc does. Disabling lccc's
  vectorizer: `struct_copy` gets **2.5× slower** (so it is a large win there),
  `fannkuch` gets **3.5% faster** (so it is a small loss there), the rest are
  neutral. One kernel losing 3.5% is a cost-model item, not a reason to touch
  the vectorizer.
- **`tls_seg_access`** (worst at 2.19×). Widening does not help: the counter
  has genuine *arithmetic* uses (`i-1`, `i&7`) feeding addressing, not just a
  cast. Extending widening to constant-offset derivatives of the IV is the
  natural next step and is filed in the backlog. lccc also emits **no `%fs:`
  segment addressing at all** here where GCC uses `tls_slots@tpoff` — worth a
  separate look.

## 5. Oracle standing (`-O3 -march=x86-64-v3`, GAS 2.47)

| function | lccc | gcc 16.2 | clang 23.1 | icx | icc |
|---|---|---|---|---|---|
| `sum_i32` | **55** | 73 | 68 | 32 | 53 |
| `sieve` | 38 | **26** | 30 | 30 | 80 |

lccc now beats GCC and Clang on the widened `int` reduction; ICX is ahead on
both and is the standing target.

---

## 6. Validation

| Gate | Result |
|---|---|
| `cargo test --lib` | **1480 pass / 0 fail / 6 ignored** |
| `./scripts/run_regression_suite.sh` | **PASS=563 FAIL=0 SKIP=15**, AB-diff 0 |
| `ir_verify_sweep.py --levels O0..Oz` | **0 violations** / 3372 configs |
| 33-kernel corpus | all 33 byte-identical; geomean 0.7431 |

---

## 7. To do next

1. **Widen constant-offset derivatives of the IV** (`i-1`, `i+1`, `i&mask`
   feeding addressing). Unblocks `tls_seg_access` and every stencil/recurrence
   loop. The IV analysis currently bails on any non-cast arithmetic use.
2. **`tls_seg_access`: no `%fs:` addressing at all.** GCC uses
   `tls_slots@tpoff(%r9,%rax,8)` and `%fs:(%r8,%rcx,8)`; lccc materializes a
   plain RIP-relative address. Worth confirming whether this is merely a
   missed addressing mode or something that matters for real multithreaded TLS.
3. **`arith_loop` 1.41×** — 32-variable register pressure; lifetime demotion
   spills whole ranges instead of splitting them (backlog RA-PRESSURE-1).
4. **`fannkuch` vectorization cost model** — 3.5% loss from vectorizing.
5. Unchanged: `PF-CLS-1` classifier chain, MachInst clobber modelling +
   `Call`, def-dominates-use in the verifier.

---

## 8. Notes

- **Probe the pass with a three-line program before believing it works.**
  `iv_widen` had tests, ran in the pipeline, and fired on roughly none of the
  loops it was written for. One probe across element widths exposed it.
- **Say which measurement supports which claim.** The corpus run and the
  same-window A/B disagree about `sieve` and both are correct; only one of
  them is an attribution.
