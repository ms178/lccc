# vec_arx — ChaCha lane-vectorizer: legality & cost-model prototype (row 17 groundwork)

Status: PROTOTYPE DESIGN, not integrated. Oracle-verified against ICX latest /
GCC 16.2 / Clang 23.1.0 at `-O3 -march=x86-64-v3` (2026-09-24,
`scripts/codegen_oracle.py`, artifacts `results/oracle-chacha/`).

## Measured baseline (chacha20_core, static insns / spills)

| compiler | insns | loads | stores | spills | best-x |
|----------|-------|-------|--------|--------|--------|
| lccc (S11) | 166 | 23 | 8 | 16 | 2.63x |
| gcc 16.2   | 237 | 61 | 54 | 72 | 3.76x |
| clang 23.1 | 187 | 45 | 29 | 26 | 2.97x |
| icx latest |  63 | 11 |  2 |  0 | 1.00x |

No compiler emits AVX2 for the *scalar-loop* source form; ICX alone recognizes
the four independent QR lanes and lowers the round function to XMM lane form.
That recognition is the exact vec_arx target: lccc can match it and beat it
(see 3-step rotate folding below), because ICX still pays 3 ops for rotl12/7
where a fused shuffle form exists.

## ICX lane form (disassembled, `results/oracle-chacha/chacha20_block-all/icx.s`)

* State = 4 XMM registers, each holding one COLUMN (lane k of all four QRs).
* Double rounds use the standard diagonalization: lane-rotate rows by
  0/1/2/3 with `vpshufd $78/$57/$147` between the column and diagonal QR
  halves — no transpose in the loop.  `vpshufd` does LANE rotation ONLY:
  it copies whole dwords and cannot mix bytes within a dword, so it can
  never implement a per-dword 16-bit rotation (earlier revisions of this
  doc wrongly claimed a fused vpshufd rotl16 — the disassembly says
  otherwise).
* `rotl16` and `rotl8` are `vpshufb` byte-LUT rotations (control vectors
  .LCPI1_0/.LCPI1_2/.LCPI1_3/.LCPI1_4) — 1 op each.
* `rotl12` / `rotl7` pay the AVX2 shift-pair (`vpsrld`+`vpslld`+`vpor`) — 3 ops.
* Loop keeps all live state in xmm5–xmm8 + 5 mask constants: zero spills.
  Exact per-double-round count from this listing: **40 vector insns** =
  8 vpaddd + 8 vpxor + 8 vpshufb + 4 shift-pairs (12 insns) + 4 vpshufd
  (lane rotations), ×10 rounds + ~20 prologue/epilogue insns.
* Epilogue: `vpshufd`+`vinserti128` widen to YMM; state += original input via
  2× `vpaddd (%rsi)` + 2× `vmovdqu` stores (the loop never re-stores state —
  the original block stays in memory and is re-read once).

## Latency, not op count, decides the round loop (2026-09-26 correction)

The op-count table below is right about instruction counts and wrong as a
performance model.  A round loop is a serial recurrence, and the benchmark
chains blocks through the feed-forward, so the time per double round is its
critical path.  Measured with `scripts/loop_latency.py` and llvm-mca 19:

| loop (cycles / double round) | znver4/5 | raptorlake/SPR |
|---|---|---|
| lccc lane form, b/c/d shuffled (before) | 30 | 31 |
| lccc lane form, a/c/d shuffled (b anchored) | **28** | **29** |
| icx `-O2 -march=x86-64-v3` (lane form) | 30 | 30 |
| gcc 16.2 / clang 23.1 `-O2 -march=x86-64-v3` (scalar, `rol`) | 24 | 32 |

`b = rotl(b, 7)` feeds `a += b` directly, so shuffling b puts one `pshufd`
per group boundary on the critical path.  The ARX passes therefore pick
lane frames with an explicit latency model (`vec_arx::kernel_critical_path`).
Scalar ARX has the lower latency bound on Zen (1-cycle `rol` vs the 2-cycle
shift-pair rotate); the lane form wins on Intel, whose `rol` issues on two
ports only.  With AVX-512VL (`vprold`) the lane form reaches 24 everywhere.

## Cost model (per QR, 4 lanes at once)

| op       | scalar (×4 lanes) | AVX2 lane form | note |
|----------|-------------------|----------------|------|
| add/xor  | 4                 | 1              |      |
| rotl16   | 4 (`rol $16`)     | 1 (vpshufb LUT) | 16-bit byte shuffle, per dword |
| rotl8    | 4 (`rol $8`)      | 1 (vpshufb LUT) | |
| rotl12/7 | 4 (`rol $n`)      | 3 (shift pair) | VPROLD is AVX512-only |

Per double round, 4 lanes: scalar = 8 QRs × 12 insns × 4 lanes = 96 insns
vs **40 vector insns** → ~2.4x op-count win before register-pressure/spill
effects (lccc currently spills 16× on this kernel); the win compounds
because the lane form removes the 16-GPR pressure entirely.  B3 lowering
MUST cost rotl16/rotl8 as vpshufb LUTs and never claim a vpshufd fusion.

## Legality conditions (to be checked at recognition time)

1. Shape: n ≥ 2 sequential ARX chains `a+=b; d^=a; d=rotl(d,K)` over
   i32 values with single-def single-use lanes inside one loop body whose
   trip count is loop-invariant (ChaCha: 10 double rounds).
2. Independence: the four chains are data-independent (disjoint value webs;
   verified by the existing def-use web partition).
3. Pure integer i32; no UB-relevant overflow handling differs (wrapping).
4. Aliasing: loop reads only the local state copy; the input block is read
   once after the loop (`vpaddd (%rsi)`) — the original memory image is
   preserved by construction, so output-aliasing-input stays legal.
5. Loop exit state materialization: widen xmm columns → ymm/gpr epilogue or
   spill-to-slot lanes; all exits (including early exits) rebuild scalar form.

## Fallback & ISA gating

* Runtime dispatch: `__builtin_cpu_supports("avx2")`-equivalent CPUID path in
  the emitter's ISA feature set; scalar fallback = today's codegen (kernel
  contexts with IRQ-off SIMD restrictions keep the scalar path — this is a
  userland-only opt-in per function/translation unit).
* Differential vectors (required before integration): per-lane symbolic
  execution of scalar vs lane form over the QR sequence; RFC 7539 test
  vector as end-to-end check; randomized state × 10^6 blocks vs the scalar
  reference implementation.

## lccc-specific integration points

* Recognition: `generation.rs` ARX-chain pass (already detects rot chains for
  the accumulator; extend to web partition across 4 chains).
* Lowering: new `isel` lane-form emitter (vpshufd/vpshufb masks materialized
  as rip-relative rodata like ICX's .LCPI constants).
* Cost gate: emit lane form only when the chain length ≥ 2 double rounds and
  x86-64-v3 is enabled; otherwise current scalar path (no regression).
