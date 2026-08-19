# 2026-08-19 (session 18) — x86-64 alu.rs audit + i686 const-division strength reduction

**Base:** `origin/main` @ `afa7a34c` (Merge PR #131 — session 17 i686 alu rewrite)
**Snapshot:** `/home/user/ms178-1.patch`

## i686: constant division/remainder strength reduction (the big win)

The IR `div_by_const` pass is **disabled on i686** (it emits 64-bit mulhi
sequences the backend truncates), so constant division stayed `idiv`/`div`
(20–90 cycles) everywhere. This session folds it in the i686 codegen using
32-bit `mull`/`imull` mulhi — the exact sequences GCC 16.2 / Clang 22.1 /
ICC / ICX emit, **verified against the godbolt oracle** (the magic values in
`magic_div.rs` reproduce GCC's immediates bit-for-bit):

| divisor | GCC immediate | magic_u32/s32 |
|---|---|---|
| u /3 | 0xAAAAAAAB, >>1 | (0xAAAAAAAB, 1, false) |
| u /5 | 0xCCCCCCCD, >>2 | (0xCCCCCCCD, 2, false) |
| u /7 | 0x24924925, add-form | (0x24924925, 3, true) |
| u /100 | 0x51EB851F, >>5 | (0x51EB851F, 5, false) |
| s /3 | 0x55555556, s0 | (0x55555556, 0) |
| s /5 | 0x66666667, s1 | (0x66666667, 1) |

Also: signed division by ±2^k (sign-splat `cltd; shrl $32-k; addl; sarl` —
one instruction SHORTER than GCC's cmov form), signed remainder by ±2^k
(matches GCC's cltd trick), and scratch-free 2-instruction LEA multiply
chains for *6/*10/*12. **All gated by `!optimize_for_size`**: -Os/-Oz keep
the shorter `idiv`/`imull` (boot `.text` unchanged at 33445), while -O1/-O2/
-O3 take the fast path (verified: `-Os` emits `divl`, `-O2` emits `mull;
shrl`).

A one-instruction bug was caught and fixed during validation: the signed
magic sign-correction used `shrl $31` (logical, 0/1) instead of `sarl $31`
(arithmetic, 0/−1), making every negative dividend come out one low
(`sd3(-9)` == −5). The per-function differential harness caught it.

## x86-64 alu.rs: verdict on the Grok report (all empirically checked)

| # | Finding | Verdict |
|---|---|---|
| 1 | neg/not ignore type → negq on U32 wrong | **AGREE, fixed** — negl/notl for I32/U32 (the register-direct paths already did this; the accumulator fallback didn't). |
| 2 | mem-dest skips reg_cache invalidation | **AGREE, fixed** — the "NO cache invalidation" comment was wrong: an in-place `op mem` leaves %rax/%rcx holding a stale copy of dest/lhs if they were cached. `invalidate_cache_for_values([dest,lhs])` added. |
| 3 | bswap I8 → bswapq | **Agree (latent), fixed** — I8 is now the identity. |
| 4 | clz/ctz/popcnt I8/I16 use 64-bit forms | **Agree (latent), fixed** — zero-extend + width adjust. |
| 5 | fused mul-add mem-source lacks alloca guard | **AGREE, fixed** — the plain binop path had the guard, the fused path didn't. |
| perf 1 | float neg 5 instr | **AGREE, fixed** — F32 is one `xorl $0x80000000,%eax`; F64 is `movabsq; xorq` (2 instr, no XMM round-trip). Bit-exact vs gcc (incl. −0.0 and NaN patterns). |
| perf 2–4 | pow2/magic div/rem missing | **Already done at IR level** (simplify.rs pow2; div_by_const.rs magic) — verified; no codegen work needed. |
| perf 6/9 | mul strength reduction incomplete | **Already partly done** (isel/`try_emit_acc_immediate`); i686 got the additions this session. |
| perf 5/7 | BMI2 shifts | Deferred — needs the march/BMI2 feature gate; documented. |

**Also rejected** (from the draft): `emit_udivrem_magic`'s partial-emit
placeholder was replaced by a clean single-path emitter; the `%esi`/`%r11`
scratch question is moot (i686 uses `pushl`/`popl` for n preservation, which
needs no allocator coordination).

## Ported to other backends (neg/not width bug class)

ARM64 (`neg w0,w0`/`mvn w0,w0` for ≤32-bit) and RISC-V (zero-extend mask
after `neg`/`not`, matching the existing `emit_bswap` convention). Not
locally testable (no aarch64/riscv64 toolchain + qemu-user here) — covered
by the CI ARM/RISC-V suites.

## Validation

- 922 unit tests (4 new magic-div tests: GCC-constant assertions + exhaustive
  divisor sweeps vs `n/d`), 50/50 correctness, **345**+6 regression (new
  `check_i686_const_div.sh`), **900-case** i686 differential fuzz (seeds
  0:300 × O0/O2/Os), x86-64 bit-exact fneg.
- Boot `-Os` `.text` **unchanged** at 33445 (opt-level gating verified).
