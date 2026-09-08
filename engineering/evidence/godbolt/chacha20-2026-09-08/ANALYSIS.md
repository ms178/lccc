# ChaCha20 oracle comparison — 2026-09-08 (post-unified-revision f7cfbd74)

Flags: `-O2 -march=x86-64-v3` (scoreboard convention). Oracle asm via
`scripts/godbolt.py compare` (artifacts in this directory; manifest records
resolved compiler ids). Runtimes measured on the Ice Lake-SP VM (taskset -c 0,
paired runs; oracle binaries assembled locally — GCC/Clang artifacts lack
their `.bss` under the godbolt filter, so local gcc-14 builds stand in for
their scalar runtimes; ICX's artifact is complete and output-verified).

| Compiler | chacha20_core insns | runtime | strategy |
|---|---:|---:|---|
| **LCCC (this tree)** | 376 | **415 ms** | scalar ARX, state in stack slots, folded memory operands |
| GCC 16.2 (oracle asm) | 195 | — (gcc-14 scalar stand-in: **278 ms**) | **auto-vectorized** (`vmovdqu .. %ymm`, vpshufd lane ops in core) |
| Clang 23.1 (oracle asm) | 187 | — (scalar-class) | vectorized forms in artifact |
| ICX latest (oracle asm) | 63 | **237 ms** (output `ff1bc75f6884e79f` verified) | **full SSE2 lane vectorization**: 4 columns in xmm, `vpshufb`/`vpshufd` lane rotations, double-round loop, ~34 insns/iteration × 10 |

## Verdict

LCCC's scalar kernel is the best SCALAR shape LCCC has produced (main: 1239 ms
→ 415 ms; gate 480→461 insns), but **all three oracles beat it**: GCC-16.2 and
Clang-23.1 auto-vectorize the quarter-round structure at `-O2 -march=v3`, and
ICX's 63-instruction kernel is the classic OpenSSL-style SIMD ChaCha. To
"beat every compiler" on this workload LCCC needs ARX lane vectorization —
a new idiom family, not a tuning of the current one.

## Why the scalar kernel sits at 415 ms (measured, not inferred)

* The 16-word state lives in stack slots; every ARX operand is a folded
  memory operand (`addl 128(%rsp), %esi`). The loop-carried chain pays
  L1 store-to-load forwarding at each round head.
* The admission cap arms at allowed=0 because the loop's block-local peak is
  17 (the scheduler interleaves the 4 independent quarter-round chains of a
  column round for ILP: 17 concurrent chain values against a 14-register
  pool).
* **The span/short split is NOT the lever**: a full homed-span sweep
  (`CCC_RA_LOOP_SPAN_RESERVE` 0/2/4/6/8/10) measures 416/449/438/416/416/423
  ms — flat. Whatever the split, some values sit in slots and the chain pays
  the same forwarding.
* The peak-17 set was identified instruction-by-instruction: rotate results
  and add results of the interleaved QR chains (genuine simultaneity, not a
  counting artifact; phi-materialized state updates were excluded and the
  peak did not move).

## Follow-up design: ARX lane vectorization (the competitive answer)

The ICX/GCC transform, in LCCC's vectorizer terms:

1. **Pattern**: an inner loop whose body is 2n quarter-round groups over 4k
   SSA state values, where the second group applies the SAME operations to a
   lane-permuted instance of the same state (ChaCha's diagonal round =
   column round with lanes rotated 1..3). Rotations must already be native
   `RotateLeft` (they are, since PR #440).
2. **Pack**: the 4 column words into one xmm at the loop head
   (`vpunpckldq`-style or vpshufd), with `vpshufb` masks standing in for the
   lane permutations between rounds (the diagonal selection).
3. **Map**: each ARX op maps 1:1 to `vpaddd`/`vpxor`/`vpslld|vpsrld|vpor`
   (or `vprold` with EVEX). The rotate-by-constant has no SSE2 form —
   lower as shift/or pairs exactly as ICX's artifact does.
4. **Unpack + add**: `vpshufd`-inverse unpack and `vpaddd` against the
   input state at the loop exit (ICX's tail).
5. **Cost model**: the scalar body must be ≥ the vector body's latency × 4
   lanes... trivially true for ARX; the gate is pattern-match success and
   AVX2/SSE2 availability.

This is the same shape family as the existing map/reduction machinery's
"same op tree applied to a permuted instance" — the permutation-aware map
parser extension is the natural implementation vehicle.

---

## Addendum 2026-09-08 (later session): rebased onto PR #448 (4f4f417d)

PR #448 landed the #442 lineage upstream directly (a superset of the port I
had carried). Red-team findings on 4f4f417d, all empirically confirmed:

| Defect in upstream main | Impact | Fix in this revision |
|---|---|---|
| cmp-replay readable() fall-through | latent miscompile (loop_rotate class) | fail-closed prune (prologue.rs) |
| FMA dominance planner lacks a GEP arm | matmul 15.1ms, FMA vectorization lost (2x) | GEP planned/cloned; matmul 5.8ms (-61%) |
| no structural auto-arm | chacha20 726ms | strict-pigeonhole arm + remcost ceiling; 414ms (-43%) |
| (refinement) my earlier valve victim guards | sha256 754ms with them | dropped — #448's span-lock valve subsumes them; 492ms (-13%) |
| dead-operand commute rule (mine, 2026-09-08) | tls_seg +24% | REMOVED — buys nothing on this tree (crc32 155.3 vs 155.4) |
| movz truncation + mask distribution absent | crc32 176.5ms | kept; crc32 152.5ms (-14%) |

Final full-corpus A/B vs 4f4f417d (40 workloads, paired runs): worst delta
+0.9% (noise); wins: matmul -61%, chacha20 -43%, tce_sum -29%,
linux_find_bit -25%, nbody -21%, expat -20%, sqlite_varint -20%,
glibc_memcmp -18%, gzip_crc32 -14%, sha256 -13%, strlen -12%, sieve -11%.
zlib_ng_adler32: instructions 240->233, stack slots 4->11, runtime flat
(its NMAX loop is a genuine strict-pigeonhole: shorts peak 10 vs a
7-register main-wave pool; baseline refreshed per the gate's contract).

The ARX lane-vectorization follow-up design below is unchanged and remains
the path to beating ICX's 237ms.
