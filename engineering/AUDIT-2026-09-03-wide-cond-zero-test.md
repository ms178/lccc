# Wide (128-bit) condition zero-test miscompile + the vector-move extent hole that masked it

**Date:** 2026-09-03 · **Base:** `110d6ad0` (Merge PR #368) + rebased S28 (`9ddfdf5a`, `c0cf9ad4`)
**Found by:** the follow-up round's i128-family sweep (12 shapes × 5 cond sources × 10 values × 5 opt levels, differential vs GCC)

## 0. Executive summary

| # | Defect | Symptom | Status |
|---|--------|---------|--------|
| A | Wide condition zero-tests examined a single 64-bit (or 4-byte) slice | `(__int128)1 << 64` selected/branched the FALSE arm (`c ? a : b` returned `b`) — in Select slot, Select accumulator, CondBranch fallback, and all legacy pushfq paths | **FIXED** |
| B | `eliminate_dead_stores` models packed vector moves as 8-byte point accesses | The i128 parameter's high-half home store was elided (its only reader: the `movdqu` slot copy); the correct wide test then read **stale frame bytes** — defect A's fix exposed this latent tomb | **FIXED** |
| C | Select stack-slot path `cmpl $0` under-read every slot wider than 4 bytes | Same disease PR #368 fixed for register-direct conditions, in its stack-slot sibling (I64-homed `1 << 32` conds) | **FIXED** (type-sized compare) |

Probe before the fix (`sel/selif/neg/usel` on `(__int128)1 << 64`): `FAIL r=201` vs GCC `OK r=0` at -O2.
After: differential sweep **MATCH at O0/O1/O2/O3/Os**; units 1679/0/6; suite PASS=576 FAIL=0 SKIP=15, AB-diff 0.

## 1. Defect A — the zero-test width

### 1.1 The semantics

A wide value is zero iff **both** halves are zero. Zero-ness is `(lo | hi) == 0`, which `orq`
computes in one instruction — GCC's canonical shape (`movq lo, %r; orq hi, %r; jcc`).

### 1.2 The four broken sites (x86-64 backend, comparison.rs)

1. **Select, stack-slot home** — `emit_cmp_zero_mem` emitted a hard-coded `cmpl $0, slot`
   (4 bytes of a 16-byte slot!). The `sel` probe compiled to `cmpl $0, 8(%rsp); cmovnel` —
   the low 4 bytes of the condition decided a 128-bit select.
2. **Select, accumulator home** — `testq %rax, %rax` (low half only; the hi half lives in %rdx).
3. **CondBranch fallback** — `operand_to_rax(cond)` + `testq %rax, %rax`: for a wide slot value,
   `operand_to_rax` loads the low 8 bytes; `selif` compiled to `movq 8(%rsp), %rax; testq %rax, %rax`.
4. **Legacy pushfq paths** (×4 sites) — same `operand_to_rax` + `testq` staging.

The register-direct paths were fixed for ≤32-bit widths by PR #368; wide types fell into the
`_ => {}` default (`testq`) — harmless only because a wide value is never single-GPR-homed.

### 1.3 The fix (comparison.rs + common.rs)

- **Branch context** — `emit_wide_cond_branch`: fold the halves into the accumulator and branch
  once, the GCC shape: slot home → `movq slot, %rax; orq slot+8, %rax`; acc-pair home →
  `orq %rdx, %rax`; anything else → `operand_to_rax_rdx` then fold. One `jcc`, existing
  hot/cold layout preserved. Clobbering rax/rdx at a block-terminating branch matches the
  generic fallback's contract (cache invalidated on exit).
- **Select context (cmov)** — `test_wide_cond_in_place`: read-only rcx-form fold
  (`movq lo, %rcx; orq hi, %rcx`) — flags must survive into the cmov, and the select flow stages
  its arms into %rcx strictly after the test, so the clobber is consumed before any live use;
  the secondary-register cache is invalidated to keep the `select_operand_to_reg` discipline.
  Slot home preferred (cache-independent), acc-pair home second, else `false` → legacy path.
- **Legacy pushfq paths** — `emit_legacy_cond_test`: wide → stage the pair + `orq %rdx, %rax`
  (the pushfq protects the flags across arm staging, so the fold's register mutation is safe);
  non-wide keeps the historical form.
- **Stack-slot width (defect C)** — the slot arm now sizes the compare from the condition's
  recorded IR type via `cmp_width_info` (`cmpb`/`cmpw`/`cmpl`/`cmpq`), through the new
  `emit_cmp_zero_mem_sized` in common.rs. Width rule is directional: reading FEWER bytes than
  the store defined is always sound (the low bytes of a stored value are always defined);
  reading MORE reads stale frame bytes. Untracked types keep the historical `cmpl`.

An intermediate two-`cmpq`+two-`jcc` form was tried and **rejected by the sweep**: a jump
placed after the low-half test is wrong under OR semantics (a zero low half does NOT imply a
zero value — the other half may be nonzero). The orq-fold is both correct and one instruction
shorter at the control-transfer level.

### 1.4 Oracle shape (GCC 14.3, -O2)

| function | lccc | GCC |
|---|---|---|
| `selif` (branch) | `movq slot,%rax; orq slot+8,%rax; je` | `movq %rsi,%rdx; orq %rdi,%rdx; jne` |
| `sel` (select) | `movq slot,%rcx; orq slot+8,%rcx; cmovnel` (branchless) | `orq` + `je` branch |

Identical fold shape; lccc's select stays branchless where GCC branches.

## 2. Defect B — the elided high-half home store (the latent tomb)

### 2.1 Discovery

After fix A, the sweep's `loopmut` shape (wide condition mutated by `c >>= 1` inside a loop)
diverged at -O2/O3 only: `loopmut(0,…)` returned +333 instead of −666 — the compiler now took
the TRUE arm for a zero condition, the *inverse* of defect A. The emitted `orq` read the
condition's slot copy high half — which contained **stale frame bytes**: the i128 parameter's
high-half home store (`movq %rsi, 104(%rsp)`) was missing from the final asm. The pre-fix
binary has the same missing store — masked for years because the old under-wide test never
READ the high half. Fix A turned the latent tomb into a live miscompile; both halves must land
together.

### 2.2 Root cause (bisection: `CCC_NO_PEEPHOLE=1` → phase 2 → `CCC_PEEPHOLE_SKIP=dead_stores`)

`eliminate_dead_stores` scans forward from each scalar stack store, looking for a read or a
full overwrite. Its read model for `Other` lines uses the cached `rbp_offset` — ONE offset
per line, checked as a POINT against the store's 8-byte range. The reader of the high-half
store was `movdqu 96(%rsp), %xmm0` — the Mov128 slot copy — whose access spans `[96, 112)`.
The point model credited only `[96, 104)`; the store at 104 read nothing → elided.

`store_forwarding` fixed the same range-vs-point disease long ago (16-byte invalidate,
"covers every x86 access width"; the i686 spectral_norm bug). `eliminate_dead_stores` kept
the point model. (The blanket alternative — marking vector moves opaque via
`has_indirect_mem` — was tried first and REJECTED by the unit suite: it kills the
`movsd`→`mulsd` memory-operand fold, `test_fp_memory_fold_mulsd`, whose scalar lines are
point-accurate and must stay precisely modelled.)

### 2.3 The fix (types.rs + dead_code.rs)

`vector_frame_move_extent(line) -> Option<i32>` in types.rs: `Some(16)` for packed SSE moves
with a frame-base operand (`movdqu/movdqa/movups/movaps/movupd/movapd/lddqu`, `v`-prefixed
AVX forms included), `Some(32)` for `%ymm` forms, `None` otherwise. Scalar-FP forms
(`movsd`/`movss`/SSE `movq`) are deliberately absent — point-accurate, and the FP folds
depend on them staying that way. String-op spellings (`movsq`/`movsl`/`movsb`/`movsw`) use
%rsi/%rdi bases and are excluded by the frame-base gate.

`eliminate_dead_stores` now overlap-checks the vector extent against the store's range before
the point check: overlap ⇒ read ⇒ store kept. The high-half home store survives; the wide
test reads defined bytes.

## 3. Validation

- Differential sweep (12 shapes × param/computed/libcall/thread-local cond sources × 10
  boundary values incl. `1<<64`, low-only, high-only, all-ones, sign bits): **MATCH vs GCC at
  O0/O1/O2/O3/Os**.
- `tests/regression/wide_cond_zero_test.c` — the sweep, codified (suite-picked; single printed
  hash).
- `tests/regression/check_wide_cond_zero_test.sh` — shape contract: branch = `movq/orq` +
  single jcc; select = rcx-form fold; no `cmpl $0` under-read; the i128 param's high-half home
  store present; dirty runtime proof for the `1<<64` class.
- Units 1679/0/6 (incl. `test_fp_memory_fold_mulsd` — the guard that rejected the opaque
  design); regression suite PASS=576 FAIL=0 SKIP=15, AB-diff 0.

## 4. Notes for future rounds

- The i128 parameter-home staging emits both halves only where nothing later overwrites the
  slot; the dead-store interaction is now pinned by the shape check. A prologue-level
  guarantee ("an i128 param home is always written in full before any read") would let the
  elision stay for the truly-dead half — current design keeps correctness local to the
  peephole instead.
- `parse_rbp_offset`'s single-offset point model is now known-incomplete in three places
  (dead_stores fixed; store_forwarding's 16-byte invalidate predates this; any future pass
  reading Other-line offsets must consult `vector_frame_move_extent` for packed moves).
