# DA-02 closure: `k_bn_mul128` runtime evidence

**Date:** 2026-11-11
**Shape:** `tests/bench/k_bn_mul128.c` (bignum 64x64->128 multiply loop)
**Harness:** `scripts/bench_kernels.py` (best-of-N wall clock, CPU-pinned, checksum-gated)
**Host:** x86-64 (Ice Lake-SP class), 3.8 GiB RAM, `/swapfile` 4 GiB active

## What the old number said

`engineering/FOLLOWUP-2026-11-09-da-audit-redteam.md` recorded DA-02 as
**open**: `lccc 2.421 ms vs GCC 0.199 ms = 12.2x slower` at `-O3`. That
measurement predates the wide-integer work in this changeset. This document
replaces the number with a same-harness measurement of the tree as it stands,
and gives the per-pass attribution so the claim can be re-checked rather than
believed.

## Result: the gap is closed, and the direction reversed

Three independent repeats, `--reps 9 --inner 1000`, arms interleaved by the
harness (relative numbers are the invariant; absolute milliseconds scale with
`--inner`):

| run | lccc (ms) | GCC 16.2 (ms) | Clang 23.1.5 (ms) | vs GCC | vs Clang |
|-----|-----------|---------------|-------------------|--------|----------|
| 1   | 0.920     | 1.042         | 0.951             | 1.13x  | 1.03x    |
| 2   | 0.969     | 0.982         | 0.957             | 1.01x  | 0.99x    |
| 3   | 0.913     | 1.186         | 0.954             | 1.30x  | 1.05x    |

(`>1` means LCCC is faster.) Checksums agree across all arms; `bench_verify`
passes, so no arm is winning by computing something different.

**12.2x slower -> parity to 1.3x faster than GCC 16.2, and parity with
Clang 23.1.5.**

## Attribution: which pass bought what

Same compiler, one pass disabled through the existing kill switch
(`--lccc-alt-env`). This isolates each pass's contribution instead of
attributing the whole delta to the changeset.

| arm | lccc (ms) | pass disabled (ms) | contribution |
|-----|-----------|--------------------|--------------|
| `CCC_DISABLE_PASSES=i128_split`        | 0.927 | 4.291 | **4.63x** |
| `CCC_DISABLE_PASSES=index_hoist`       | 0.985 | 3.689 | **3.74x** |
| `CCC_DISABLE_PASSES=i128_narrow_mul`   | 0.916 | 0.920 | 1.00x (not exercised by this kernel) |

`i128_split` removes the 16-byte slot round-trip per loop-carried 128-bit
value; `index_hoist` is what lets the induction-variable widener see a plain
`base[j]`, without which the split's work does not survive to the inner loop.
The narrow-multiply plan is genuinely absent from this kernel (neither operand
is a 64-bit-extended value), which is why it measures as a no-op here — that is
the honest result, not a defect: the plan exists for shapes like
`(u64)a * (u64)b` widened to 128 bits, and its own tests cover it.

## Reproduce

```sh
scripts/restore_agent_toolchain_cache.py --only rust,clang,gcc,gdb,binutils
source ~/.cache/lccc/agent-toolchains/env.sh
scripts/build_lccc_fast.sh
python3 scripts/bench_kernels.py --kernels bn_mul128 --reps 9 --inner 1000 \
    --lccc target/fastbuild/lccc --gcc "$(command -v gcc-16)" --clang "$(command -v clang-23)"
python3 scripts/bench_kernels.py --kernels bn_mul128 --reps 9 --inner 1000 \
    --lccc target/fastbuild/lccc --lccc-alt-env CCC_DISABLE_PASSES=i128_split
```

## What this does NOT claim

* Parity on the *whole* kernel suite. Measured with the same harness at
  `-O3`, the suite geomean is `0.86x` GCC: several shapes remain slower, and
  the widest of them are named below.
* A closed RAP-04/RAP-05 allocator programme. The allocator still parks
  `I128`/`U128` in slots (`src/backend/regalloc.rs`); what changed is that the
  *dynamic multiply path* no longer pays a round-trip per iteration.

## The next shape, measured and root-caused

Full suite, `--reps 7 --inner 1000`, `-O3`, relative to GCC 16.2 (`>1` is
LCCC faster):

| kernel | lccc (ms) | GCC (ms) | vs GCC | note |
|--------|-----------|----------|--------|------|
| `classify`     | 8.650 | 1.669 | 0.19x | multi-range byte classification |
| `namechars`    | 6.707 | 1.677 | 0.25x | same predicate, counting form |
| `memchr`       | 7.586 | 5.611 | 0.74x | Clang vectorises, both scalars |
| `varint`       | 3.600 | 2.759 | 0.77x | |
| `sqlite_varint_scan` | 1.515 | 1.083 | 0.71x | |
| `seg_gs_copy`  | 2.234 | 1.628 | 0.73x | |
| `adler32`      | 2.221 | 1.879 | 0.85x | |
| `strcmp_signed`| 22.111 | 19.075 | 0.86x | |

The classification shapes are the widest gap and the cause is measured, not
guessed. The predicate

```c
static int is_name_char(char ch) {
    return ((ch >= 'a' && ch <= 'z') || (ch >= 'A' && ch <= 'Z') ||
            (ch >= '0' && ch <= '9') || ch == '_' || ch == '-' || ch == '.');
}
```

compiles to a **partially** if-converted loop: `orl` of the two letter-range
`setcc` results, then a data-dependent `je`, while the digit/equality half is a
`Select` chain. The loop vectorizer bails on that residual branch and emits
scalar code (~30 instructions per byte). Measured proof that this — and not the
absence of a better peephole — is the whole gap: rewriting the *same* predicate
with bitwise `|` instead of `||` (a source change, not a compiler change) makes
the loop vectorize and runs **5.2x faster**:

| variant | lccc | GCC 16.2 | checksum |
|---------|------|----------|----------|
| `k_classify.c` as written            | 0.017190 s | 0.003319 s | 5182000 |
| same predicate with `\|` (branchless) | 0.003301 s | 0.008869 s | 5182000 |

So the remaining work is precise: make the short-circuit chain
(`a||b||c||d||e||f`) lower to the branchless form the vectorizer accepts, at
which point LCCC reaches `0.0033 s` — parity with GCC's own best arm. The
mechanism is `if_convert`'s `tree-ssa-ifcombine` equivalent: it accepts at most
one chain level per round, and its profitability simulation asks "does one
round make the body branch-free?", which a six-link chain never does
(`CCC_DEBUG_IFCOMBINE=1` shows `reject (a branch in the body survives the
combine)`). Deepening that simulation in this changeset did **not** create new
candidates — the surviving blocks are join blocks with several predecessors,
not chain links — so the change was reverted rather than landed as unproven
complexity. The transform that would work is a chain-aware flattening pass,
which is scheduled work, not a slice to fake.

## Diagnostics added by this changeset

| knob | effect |
|------|--------|
| `LCCC_TYPEOF_RECOVERY_TRACE` | Every `typeof` that falls back to the recovery-only `CType::Int` prints `typeof-recovery: ...` naming the operand. Off by default; a successful compile prints nothing. Used by `tests/corpus/test_typeof_unresolved_fallback.py` to check the contract in both directions. |
| `CCC_DISABLE_PASSES=index_hoist,i128_split,i128_narrow_mul` | Existing kill switch, used above for pass attribution. |

## The classification loop's second defect: constant pressure in the vector path

The branchless experiment above shows the *scalar* loop is what costs 5.2x. It
also exposed a second, independent defect that survives vectorization: the
packed classifier spills vector temporaries to the stack because the constant
splats occupy the register file.

Measured on the vectorizing branchless kernel (`-O3`, one function, counted
from the emitted assembly):

| predicate | constants | vector spills | vector reloads | packed ALU ops |
|-----------|-----------|---------------|----------------|----------------|
| `[a-z] \| [A-Z]`                       | 2  | 0  | 0  | 15 |
| full `is_name_char` (6 ranges/eq tests) | 13 | 12 | 18 | 28 |
| same, `-mavx2`                          | 13 | 23 | 7  | 130 ymm uses |

The two-constant row is the controlled experiment: with room in the register
file the same loop spills nothing. So the constraint is *constant register
pressure*, not an inherent allocator limitation — and it is avoidable, because
every packed ALU op the classifier uses (`pcmpeqb`, `pcmpgtb`, `pand`,
`pandn`, `por`, `paddb`, `psadbw`) accepts an `m128` operand, and the
`.LCVEC` pool entries are produced as 16-byte aligned splats
(`.p2align 4` + two `.quad`s), so they are legal aligned memory operands.
`X86Codegen::lane_const_rip_operand` / `vec_const_rip_operand` already emit
that exact form for the unsigned-compare bias, so the mechanism is in-tree and
in use; what is missing is folding a const-pool *load* into its single
consumer instead of keeping the constant resident in a register.

That fold is the next scheduled change. It is a modification to
`vector_load_op_fuse.rs` (which already owns single-use vector-load folding,
alignment proof and a `CCC_PEEPHOLE_SKIP` switch), not a new pass. It is not
landed here: the pass's soundness rests on a width model, an alignment proof
and a liveness-monotonicity argument that a RIP-relative source extends, and
landing a half-verified version of that would be exactly the kind of
unmeasured change this document exists to prevent.

What is landed is the ratchet: `tests/corpus/test_classify_predicate_codegen.py`
records the current spill/reload counts (12/18), asserts the branchless form
reaches the vectorizer at all, asserts both predicate forms agree with GCC and
Clang on every byte at `-O0` and `-O3`, and asserts the kill switches bisect
performance but never the result.
