# ZERO-REM-1 — omit provably-dead vectorizer remainder loops

**Status:** implemented and gated (`vec-dead-remainder`, `ci_local.sh` + `.github/workflows/ci.yml`).
**Baseline:** upstream `main` `93f2a43b` — the S01–S04 work is already merged
upstream, so this change sits directly on `main`.
**Host:** 2-vCPU Xeon KVM, 1.9 GiB RAM, 8 GiB swap, fastbuild `-O1 -j2`.

---

## 1. What was wrong

The map vectorizer (`src/passes/vectorize.rs::transform_map_vector`) rewrites an
elementwise loop `for (i = 0; i < N; i++) d[i] = f(s[i])` into

1. a packed loop covering `floor(N / W) * W` elements, and
2. a **scalar mirror** loop covering the `N % W` tail.

The mirror has a second job: it is the fallback target of the runtime dependence
guards — a stream whose object root may alias the destination re-enters the loop
scalar-wise through the mirror, so no second copy of the loop body is needed.

When `N` is a compile-time constant that the packed body covers exactly
(`N % W == 0`), the mirror cannot execute a single iteration. It was emitted
anyway:

```asm
    xorl  %edx, %edx
    cmpq  $64, %rdx
    jge   .LBB3
.LBB2:                              # packed loop: 2 iterations
    vmovdqu (%rsi,%rdx), %ymm2
    vmovdqu %ymm2, 8(%rsp,%rdx)
    addq  $32, %rdx
    cmpq  $64, %rdx
    jl    .LBB2
.LBB3:
    shrq  $2, %rdx                  # ─┐
    movslq %edx, %r9                #  │
    leaq  0(,%r9,4), %r11           #  │ resume-index computation
    leaq  8(%rsp), %rcx             #  │ for a loop that runs
    addq  %r11, %rcx                #  │ ZERO times
    movq  %rcx, %r10                #  │
    leaq  (%rsi,%r11), %rdx         # ─┘
    cmpq  $16, %r9
    jge   .LBB6                     # 16 >= 16: always taken
.LBB5:                              # dead scalar loop
    movl  (%rdx), %eax
    movl  %eax, (%r10)
    addq  $1, %r9
    leaq  4(%r10), %r10
    leaq  4(%rdx), %rdx
    cmpq  $16, %r9
    jl    .LBB5
.LBB6:
```

17 instructions of dead code per vectorized loop — a guard, the resume-index
arithmetic, two materialised stream pointers and a whole scalar body. It is
general, not sha256-specific: **every** map-vectorized loop with a constant trip
count that is an exact multiple of the packed width paid it.

## 2. The fix

`transform_map_vector` now omits the mirror when it provably cannot run. Three
preconditions, each checked where the fact is known:

| Precondition | Why it is required |
|---|---|
| `pattern.limit` is a constant `N` with `N % packed_width == 0` | the only way to know the tail is empty; a dynamic `N` needs the mirror |
| `pattern.guarded_streams.is_empty()` | the mirror is the dependence guard's fallback; with guards it is live code |
| escaping uses of the counter can be re-pointed | the packed loop counts PACKED iterations, so a use outside the loop must see the element trip count |

The third point is the one that needs work rather than a check: when the mirror
exists, escaping uses of the scalar counter are rewired to the mirror's IV
(`rewire_escaping_iv_uses`). Without a mirror there is no such value, so the
transform materialises the trip count in the preheader — using the same
`Cast`-to-`iv_ty` form the mirror uses for its own resume index, so the value
keeps its type in every use position, including the bare-`Value` slots
(Store/GEP pointers) that an `Operand::Const` could not occupy:

```rust
func.blocks[preheader_idx].instructions.push(Instruction::Cast {
    dest: trip_val,
    src: Operand::Const(IrConst::I64(trip)),
    from_ty: IrType::I64,
    to_ty: pattern.iv_ty,
});
changes += rewire_escaping_iv_uses(func, pattern.iv, Some(trip_val), ...);
```

Nothing is inserted when no use escapes, so the transform stays idempotent.
Values other than the counter that cross the loop boundary are unaffected: the
existing transform only *relabels* the loop-side incoming edge of the exit
block's phis to the mirror header and keeps the vector-computed value. With a
zero-iteration mirror those are the same value, so skipping the mirror cannot
change them. (`analyze_map_pattern` already rejects loops with any other escaping
definition via `loop_escape_closed`, and rejects loops whose IV does not start
at constant 0, which is what makes `limit` the trip count.)

## 3. Evidence

### 3.1 Instructions removed

| Case | before | after | Δ |
|---|---:|---:|---:|
| `sha256_transform` (`tests/benchmark/programs/sha256_transform.c`, `-O2`) | 151 | **135** | −16 |
| — register-to-register moves / stack references | 20 / 1 | 19 / 0 | −1 / −1 |
| 16-element `unsigned` copy + increment (`copy16`, `-O2`) | 61 | **28** | −33 |
| `shape_u32_exact` (32 × `u32`, `-O2`) | 39 | **18** | −21 |

(Numbers are an A/B against `CCC_NO_MAP_ZERO_REM=1` on upstream `main`
`93f2a43b`, so they measure this change alone and not the base tree's drift.)

Measured on upstream `main` `93f2a43b`. The pinned-oracle numbers for the same
function (GCC 16.2 = 142, Clang 23.1 = 129) are the remaining target and are
taken from the Godbolt oracle, not from this host's gcc 14.2.0-19 / clang
19.1.7.

### 3.2 The mechanism fires, and only where it may

`tests/regression/check_vec_remainder_shapes.py` counts the loops of every shape
in `tests/regression/vec_shapes/vec_dead_remainder_shapes.c` as strongly
connected
components of the emitted CFG — *not* as backward branches, which are unreliable
because lccc lays the packed loop, the exit block and the mirror out in an order
that makes some logically-forward edges point at lower addresses:

| shape | `-march=x86-64` (SSE2) | `-march=x86-64-v3` (AVX2) |
|---|---:|---:|
| `shape_u32_exact` (32 % 4 = 32 % 8 = 0) | 1 loop, 18 insns | 1 loop, 18 insns |
| `shape_u8_exact` (64 % 16 = 64 % 32 = 0) | 1 loop, 18 insns | 1 loop, 18 insns |
| `shape_u64_exact` (8 % 2 = 8 % 4 = 0) | 1 loop, 21 insns | 1 loop, 19 insns |
| `shape_i32_exact`, `shape_f32_exact`, `shape_f64_exact` | 1 loop | 1 loop |
| `shape_two_src_exact` (two source streams) | 1 loop | 1 loop |
| `shape_escape_exact` (counter escapes) | 1 loop | 1 loop |
| `shape_u32_tail` (33 elements) | **2** loops, 39 insns | **2** loops, 39 insns |
| `shape_u8_tail`, `shape_u64_tail`, `shape_f32_tail`, `shape_u16_tail` | 2 loops | 2 loops |
| `shape_alias_exact` (may-alias, guard fallback) | **2** loops | **2** loops |

A shape that stops vectorizing is a FAIL, not a skip: `packed == 0` would make
"1 loop" true of a scalar loop as well. Two shapes the map analyzer does not
handle (32 × 16-bit elements, and the non-unit-stride `s[i + 1]` stream) are
listed explicitly and asserted to stay scalar, so the gap is documented and any
change to it is visible.

### 3.3 Correctness

`tests/regression/check_vec_dead_remainder.sh` (gate `vec-dead-remainder`):

* `vec_dead_remainder.c` — 20 shapes (exact multiples and their `W+1`
  neighbours for `u8`/`u16`/`u32`/`u64`/`i32`/`float`/`double`, two-source
  streams, escaping counters in `int`/`long long`, may-alias call sites at
  dependence distance 0/+1/−1, dynamic trip counts including 0 and 1, and a
  nested inner map) folded into one rolling checksum over every produced byte.
  It must agree with the GCC oracle **and** with lccc compiled with
  `CCC_NO_MAP_VEC=1` (the vectorizer's own scalar path) at
  `-O0/-O1/-O2/-O3/-Os` × `{x86-64, x86-64-v3, default}` × `{lccc, scalar}`.
* i686 (`-m32`) has no map-vectorizer lowering; the corpus must still match the
  oracle there (SKIP when no 32-bit multilib is installed).
* The loop census of §3.2.

Both oracles agree on all arms: `aebddc433b9bfbf6`.

## 4. Kill switch

`CCC_NO_MAP_ZERO_REM=1` restores the previous behaviour (always emit the
mirror). It exists so the win can be re-measured at any time without a rebuild
of a historical tree — the same contract as `CCC_NO_MAP_VEC`.

## 5. Not done here

The reduction and stencil vectorizers build their own tails; the same
constant-trip-count reasoning applies but is not implemented. The map path was
chosen first because it is the one that fires on the benchmark corpus
(`scripts/stack_census.py`/`ra_quality_census.py` kernels are dominated by
elementwise loops).
