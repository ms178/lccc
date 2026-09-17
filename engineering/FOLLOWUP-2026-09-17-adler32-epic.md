# FOLLOWUP 2026-09-17 — Adler-32 loop epic + counting-epic miscompile fixes

Session 51. Base: upstream `main` = `3041a9a2` (merge PR #546 — the v7
wave). The deliverable is the Adler-32 loop epic (the §3.1 marquee
feature nobody has) plus five latent miscompiles the epic's red-team
battery exposed in the LANDED counting epic and the unroller.

## 1. The Adler-32 rolling-checksum epic

### 1.1 What it does

The serial two-accumulator checksum

```c
while (n >= K) {            /* K ∈ {1, 2, 4, 8, 16, 32} */
    s1 += p[0]; s2 += s1;   /* K per-byte steps in one body */
    ...
    p += K; n -= K;
}
```

— which GCC 16.2, Clang 23.1 and ICX (2026-09 CE pins) all leave
scalar (verified: pure scalar unroll at every -O level) — is
vectorized 32 bytes per iteration with EIGHT instructions:

```asm
vpaddq    %ymm3, %ymm5, %ymm5      # vs3 += vs1  (the cross-term snapshot)
vmovdqu   (%rdi), %ymm0            # vbuf
vmovdqu   %ymm0, 416(%rsp)         # vbuf staged for the sad's fold
vpmaddubsw %ymm7, %ymm0, %ymm0     # vshort = (32-l)·b_l pair weights
vpmaddwd  %ymm8, %ymm0, %ymm0      # vsum2  = dword lane sums
vpaddd    %ymm0, %ymm4, %ymm4      # vs2   += vsum2
vpsadbw   416(%rsp), %ymm6, %ymm0  # vsad  = per-8-byte partial sums
vpaddq    %ymm0, %ymm3, %ymm3      # vs1   += vsad
```

plus the exit materialisation `s1 = (u32)hsum(vs1)`,
`s2 = (u32)hsum(vs2) + ((u32)hsum(vs3) << 5) + s2_init` — the
deferred `32·Σ S_k` cross term.  Everything is register-homed: the
accumulators vs1/vs2/vs3 stay in their YMM homes across the loop
(in-place three-operand VEX updates), the zero/weights/ones tables in
ymm6/ymm7/ymm8, and the only slot traffic is vbuf's own staging store
that the sad folds back as its r/m operand.

**Measured (4 MB buffer, 16 passes, this sandbox): lccc 0.200 ms/pass
= 20.0 GB/s; GCC -O2 2.89 GB/s, GCC -O3 -march=x86-64-v3 2.81 GB/s,
GCC -O3 2.51 GB/s — a 6.9–8.0× win with bit-identical checksums.**

### 1.2 Exactness (the ring-homomorphism proof, in the source)

Every term is +/× mod 2^32; the regrouping
`Δs2 = 32·S + Σ_l (32−l)·b_l` per 32-byte chunk is a ring
homomorphism, so the vector result is congruent to the scalar WRAPPING
result for every input and length.  The u64 lanes of vs1/vs3 wrap mod
2^64 and 2^32 | 2^64; the maddubs word ≤ 2·32·255 = 16320 < 2^15 (no
saturation); zero vector iterations are the hsum identity — no guard,
no merge phi.  The remainder is the ORIGINAL loop (the transform
prepends the vector loop and rewires the header phis); because K | 32
the original loop always sees a non-negative count it can finish
exactly.

### 1.3 Recognition grammar (fail-closed)

A natural loop with: straight-line discipline; two U32 header phis
closing `s1' = s1 + zext(u8 load)` and `s2' = s2 + s1'` (K ≥ 1 steps,
either operand order for the s2 add); a byte-counter phi (I32/U32/
I64/U64) stepping −K with a `n >= K` header compare; a byte-cursor
phi (I64 arithmetic or Ptr GEP family) whose loads read p+0..p+K−1 in
chain order and whose backedge is p+K; no stores/calls/volatile; a
strict body grammar (chain + casts + cursor arithmetic + counter step
ONLY, every member tracked explicitly); no body value live-out; and
an EXACT header-phi census (only the four phis the transform rewires —
a fifth phi's preheader-labeled incoming would dangle).

### 1.4 New intrinsics

`VecMaddubsU8x32` (vpmaddubsw; NON-commutative — args[0] unsigned
bytes, args[1] signed weights; a swap feeds the stream to the signed
multiplier = silent miscompile class, so operand order is preserved
structurally), `VecMaddwdI16x16` (vpmaddwd; same order discipline),
`VecConstI8x32` (32 const bytes → the shared .LCVEC rodata pool;
non-constant args panic loudly).  Both madds route through
`emit_avx_binary_256` with `memfold_consumer_256(Some(false))` (the
src2/weights slot only).  Regalloc: maddubs result/const tables are
class 8 (the I64x4 YMM family — the Copy-web homes them), maddwd
result class 5 (feeds the I32x8 vs2 accumulator); both are legal
class-8 consumers (home-aware emitters).

### 1.5 Homing: three defects found and fixed

The first epic build paid a 32-byte slot round-trip per accumulator
per iteration.  Root causes, in discovery order:

1. **`VecZeroI32x8` zero** (the previous session's uncommitted find):
   the accumulator web mixed a class-5 zero with class-8 adds — no
   unified web, no home.  Fixed: the zero is `VecZeroI64x4`.
2. **`VecHorizontalAddI64x4` poisoned every vector web**: the
   reduction-collector's whitelist had the I32x8/I64x2 horizontal adds
   but not the I64x4 one — the epic's exit hsum evicted EVERY web in
   the function to protected slots (the counting epic had the same
   latent defect: `vpaddq 176(%rsp)` round trips in its gate kernel).
   Fixed: whitelisted (its vextracti128+vpaddq+vpshufd chain confines
   scratch to the reserved pair and only reads homes).
3. **The epic's invariants (zero/weights/ones) had no home path**: not
   phi-web members (loop-invariant, not carried).  Fixed: the
   map-broadcast collector gained class 8 — `VecZeroI64x4` and
   `VecConstI8x32` whose only consumers are the epic's packed ops
   (sadbw/maddubs/maddwd/I64x4 arith).

Plus the emitter refinement: a REGISTER-HOMED args[1] with a folded
args[0] is the VEX.vvvv source directly (no %ymm1 staging copy) —
one instruction off every such iteration.

## 2. The counting-epic miscompile fixes (all latent, all live)

The battery's development shapes exposed five defects in the LANDED
byte-count epic (v6, PR #544) and the unroller — each reproduced on
the clean base:

1. **Multi-accumulator loops miscompiled**: `a += pred1(s[i]);
   b += pred2(s[i])` — the analyzer matched a's add, the transform
   replaced the loop's trip structure wholesale, and the b chain had
   no surviving definition (b stayed 0 for every n).  Fixed with the
   header-phi census: the header must carry EXACTLY the IV and the
   accumulator.
2. **IV live-out (unguarded)**: `for (i=0;...) ...; use(i)` read the
   VECTOR loop's divided IV — ⌊n/32⌋ instead of n.  Fixed with the
   limit-based materialisation (below).
3. **IV live-out (guarded)**: the same, through the exit merge phi.
4. **Narrow-compare constant wrap**: `c += (buf[i] == 0xAA)` declined
   — the frontend's narrow-compare fold wraps 0xAA to I8(−86), whose
   range fails the unsigned-domain side-condition.  Fixed: a
   single-point CONSTANT is reinterpreted modulo the other side's
   byte domain (and the tree's invariant rewritten so the packed splat
   AND the scalar mirror both see the domain value).  `== 0x5A`
   worked before; `== 0xAA` silently stayed scalar.
5. **The unroller's split-exit IV hole**: `do_unroll` threads exit
   phis with per-exit-check IV values (Step 5), but a phi over the IV
   in any block OTHER than the direct exit (an exit block an
   intermediate pass SPLIT — exactly the counting epic's remainder
   merge) keeps its stale header-edge incoming while the new
   exit-check edges bypass it: the reader sees ⌊n/k⌋·k instead of n.
   Fixed BOTH ways: the epic's final IV is now computed FROM THE LIMIT
   (unsigned: the limit itself; signed: max(limit, 0) via
   compare+select in the vec-exit) — a loop-invariant pure function of
   the source semantics that no downstream restructuring can
   desynchronise — AND the unroller gained the fail-closed 8b gate
   (an IV-phi reference in a foreign phi declines the unroll).

Also fixed: the epic's exit block defined `s2_res` TWICE (an SSA
violation the suite's ir-verify gate caught — the second add now
uses a fresh `s2_final`).

## 3. Verification

- New gate `vec-adler-epic` (ci_local.sh + ci.yml): the battery
  (7 positive shapes × 5 valid inits × every length 0..700 + long
  lengths; 8 adversarial rejections with exact sequential mirrors; 8
  counting-fix shapes) + tri-config differential (epic on /
  -mno-avx2 / gcc) + asm contracts (the three packed producers fire;
  NO YMM slot store inside the vector loop; the hsum exit chain).
- All 8 SLP/adler gates PASS; full regression suite PASS=713 FAIL=3
  (the pre-existing i686 multilib-header environmentals) SKIP=16,
  AB-diff 0; cargo test 2861/0; benchmark output oracle 204/204;
  `ci_local.sh --fast` 42 passed / 0 failed / 3 skipped; rustfmt and
  clippy (all-targets, -D warnings) green; zero warnings.
- The codegen-quality baseline refreshed with `--update-baseline`
  (zlib_ng_adler32: +50.9% insns — the vector loop + remainder mirror
  now live in the function; the RUNTIME is the point: see §1.1).

## 4. Open follow-ups (priority order)

1. **DO32 nested-loop spelling**: `while (n >= 32) { for (k=0;
   k<32; k++) { s1 += buf[k]; s2 += s1; } ... }` stays scalar — the
   outer body contains the inner loop (the strict one-body-block
   grammar declines), and the inner constant-trip loop is itself not
   complete-unrolled (its carried phis escape to the outer loop).
   Two independent enablers, either suffices: (a) teach the adler
   matcher the loop-nested chain (walk the inner loop as a symbolic
   K-chain), or (b) fix the complete unroller's inner-loop decline so
   the existing matcher sees the straight-line K=32 body.
2. **Cross-block SLP, full** (phi seeds, call-arg seeds) — the §3.1
   follow-up list item 6, unchanged.
3. **nbody scatter + dot** (loop epic).
4. **Broadcast CSE across seeds** (the pressure shape).
5. **Legacy-SSE/VEX mixing** under AVX2 (transition penalties).
6. **Dead frame slots** for demoted shapes.
7. The epic's 8-instruction body has one remaining staging store
   (vbuf's slot store feeding the sad's fold).  A scheduler that
   keeps BOTH of vbuf's consumers in registers (or folds the LOAD
   into both consumers as r/m) would reach the 6-instruction form —
   the memfold infrastructure exists (VLFOLD); the epic's IR emission
   order already minimises it to one store.

## 5. Reproducing

```
cargo build -j1 --profile fastbuild --locked
LCCC_BIN=target/fastbuild/lccc bash tests/regression/check_vec_adler_epic.sh
LCCC_BIN=target/fastbuild/lccc bash scripts/run_regression_suite.sh
bash scripts/ci_local.sh --fast
LCCC_DEBUG_VEC_ADLER=1 ./target/fastbuild/lccc -O2 -march=x86-64-v3 -S <file.c>
```
