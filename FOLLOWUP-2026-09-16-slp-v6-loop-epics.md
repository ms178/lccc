# FOLLOWUP 2026-09-16 — The Loop Epics: adler32 DO8 and nbody (design)

Session 50 / wave W6. Charter item: *adler32 DO8 / nbody — loop epics
needing reassociation, explicitly outside BB-SLP's reassociation-free
charter.* This document is the engineering design for both epics: what
they are, why the current passes stop where they stop, and the exact
plans — with the soundness proofs, the lowering shapes, and the test
contracts — for the loop-vectorizer work that would deliver them.

The S06–S09 infrastructure this design depends on (vecreg homing
completion, the bank-precise `vec_live_regs` claim discipline, the
deferred-dest-store survival fix, the 128-bit VEX memfold) is merged and
gated; every packed shape this document proposes flows through those
hardened paths.

---

## 1. Where the epics stand today (measured)

### 1.1 adler32 DO8 (zlib's classic 8-byte unroll)

```c
while (len >= 8) {
    s2 += s1 += buf[0];  s2 += s1 += buf[1];
    ...                   s2 += s1 += buf[7];
    buf += 8; len -= 8;
}
```

Current lccc `-O2 -march=x86-64-v3`: a clean **scalar chain** — 8×
(`movzbl` + `lea/add` for s1 + `add` for s2), 3 insns per byte, one
loop-carried dependency of length 16 adds per iteration. GCC 14.2 `-O2`
emits the same scalar shape (verified; GCC does not vectorize this
either — only zlib's *hand-written* `adler32_ssse3` gets the packed
form). **The epic is beating both compilers with the auto-vectorized
form zlib-ng wrote by hand.**

### 1.2 nbody force loop

The leaf `dx*dx + dy*dy + dz*dz` is already at exact GCC parity
(3×`vsubsd` + `vmulsd` + 2×`vfmadd`), and the W3 cmp+blendv/min/max and
W5 homing work cover the pair-loop's straight-line bodies. The epic is
the **j-loop's force accumulators** (`fx/fy/fz += m_j·dx/r³`), which are
FP reductions over a recurrence (`r` depends on the running pair), and
the **velocity update** (`vx += dt·fx/m` chains). Current: scalar FMA
chains, one per component.

---

## 2. Why BB-SLP cannot do these (and must not)

BB-SLP's contract, unchanged since v1: it packs **isomorphic
straight-line lanes within one basic block without reassociating
anything** — the pack is a bit-exact restatement of the scalar lanes
(every fold proven lane-exact: the FP-Neg sign-mask proof, the IEEE
blendv predicate proof, the integer min/max commutativity proof).
Both epics violate a different precondition:

* **adler32 DO8** is *not* a multi-lane isomorphism at all — it is ONE
  lane (s1, s2) whose *values* must be reassociated across unrolled
  iterations. No amount of lane-packing expresses
  `s2 += Σ w_i·b_i + 8·s1` without changing the summation ORDER.
* **nbody's** dot leaf packs fine at the leaf level in isolation (three
  squares into a horizontal sum — a 3-lane non-power-of-2 shape), but
  the *reduction* (`fx += …`) is loop-carried, and the dx/dy/dz values
  are born from loads whose addresses the pair-loop updates — a
  cross-iteration dependence web.

Reassociation of **integer** addition is exact (associative and
commutative mod 2^32, and C defines unsigned wraparound). Reassociation
of **floating point** is NOT bit-exact and requires the user's
permission (`-fassociative-math` / `-ffast-math`). These are the two
different contracts the design below honors.

---

## 3. Epic A — adler32 DO8: exact integer reduction reassociation

### 3.1 The mathematics (the whole soundness story)

All arithmetic is `uint32_t` (defined, associative, commutative,
distributive mod 2^32 — no overflow UB in sight, so reassociation needs
no range proof for *correctness*; the bound analysis in §3.3 is only
about keeping the intermediate sums in a form the packed ops can hold).

For a block of N input bytes b_0..b_{N-1} with the recurrence
`s1_{i+1} = s1_i + b_i`, `s2_{i+1} = s2_i + s1_{i+1}`:

```
s1_N = s1_0 + Σ b_i
s2_N = s2_0 + N·s1_0 + Σ (N − i)·b_i        (weights N..1)
```

*Proof*: unroll s2's recurrence — each b_i is added into s2 once per
later iteration (N−i times), and s1_0 is added N times. Both sides are
the same multiset of addends; unsigned addition's AC properties make
the reordering exact. ∎

### 3.2 The packed decomposition (the zlib-ng shape, auto-derived)

Per 16-byte block (SSSE3) or 32-byte block (AVX2), with `vs1`, `vs2`
held in a ymm:

```
vsad  = vpsadbw(block, 0)          ; 8×(16-bit) byte sums   (2×64-bit for AVX2)
vs1  += vsad                        ; exact: s1's block-total (§3.1 first line)
vsmad = vpmaddubsw(block, W)        ; signed×unsigned weighted pairs, W = 16..1 (bytes)
       — CAREFUL: vpmaddubsw saturates; see §3.3 for why 16-bit lanes hold.
vs2   = vs2 + N·vs1_prev + widen(vsmad)   (vpmaddwd with 1-splat + vpaddd)
```

The lccc infrastructure this needs, all of which EXISTS today:

| need | where it lives today |
|---|---|
| `vpsadbw` reduction | `VecSadbwU8x32` (counting battery, v5) |
| `vpmaddubsw`/`vpmaddwd` chains | user-intrinsic paths; **map-vectorizer families needed** |
| loop-carried ymm home | reduction vecreg scan + W5 claim discipline |
| stream load folding | VLFOLD 256-bit (W1) |
| 5552-block bound | the front-end's existing `% BASE` peeling (nothing new) |

### 3.3 The 16-bit lane bound (the one real analysis)

`vpmaddubsw` computes *signed* 16-bit products of *unsigned* bytes with
*signed* weights and **saturates**. zlib's decomposition keeps every
partial sum bounded because within one 5552-byte block:
`s1 ≤ 65520 + 5552·255 < 2^24` and each weighted pair-sum
`Σ_{pair} w·b ≤ (16+15+…+1)·255 = 136·255 = 34 680 < 2^15`? — no:
`saturating pairs` — zlib splits the weight vector so each *pair*
(2 bytes × weight ≤ 16+15) stays ≤ (16+15)·255 = 7 905 < 32 767 ✓ and
the maddwd accumulate adds at most 4 such pairs per dword before the
next vs2 add, and vs2 itself stays ≤ 65520 + 5552·(65520+5551·255)
< 2^31 ✓ (the classic 5552 = largest N with N·(N·255 + 65520) +
5552·65520 < 2^32). The design's proof obligation at implementation
time: emit the bound check ONCE at loop entry (peel to a multiple of
the block width, verify `len ≤ 5552` blocks between `% BASE`
reductions — exactly the structure the C source already guarantees),
and keep the pair-splitting weights as constants in the vector const
pool (the W2 `.LCVEC_` machinery).

### 3.4 The pass design (where it lives, how it's gated)

* **Location**: `vectorize.rs` — a new recognition in the reduction
  scan: a two-recurrence chain (`s1 += b` feeding `s2 += s1`) over a
  unit-stride byte stream. The recognizer emits the §3.2 decomposition
  directly as `Vec*` intrinsics (adding `VecMaddubsI8x16/32` +
  `VecMaddI16x8/16` producer families — the W5 pattern: same emitters
  the user intrinsics already use, so the class tables admit them).
* **Exactness contract**: bit-identical to the scalar chain for EVERY
  input — testable by the existing differential oracle (add an adler32
  case with adversarial lengths: 0..64, 5551..5553, 2^16±1, the
  `% BASE` boundary crosses).
* **Gating**: `-O2` default ON (integer, exact — no fast-math needed).
  Kill switch: `CCC_NO_ADLER_REASSOC`.
* **Cost model**: fires only when the stream is provably
  unit-stride + `restrict`-clean (the rule-(e) hazard machinery) and
  the loop is the hot inner loop (the existing loop-header heuristics).
* **Expected result**: 32 bytes/iteration at ~11 packed ops vs 96
  scalar ops — the measured zlib-ng SSSE3-vs-scalar ratio (~6–8×) on
  the 14700KF's 2×256-bit ports.

### 3.5 What is deliberately NOT attempted

Unrolling the *remainder* loop, and the `s1 %= BASE` epilogue: the
scalar remainder is ≤ 7 bytes (a peeling decision, not a reassociation);
the modulo stays scalar exactly as zlib keeps it.

---

## 4. Epic B — nbody: the fast-math-gated FP reduction

### 4.1 The contract difference

Floating-point reassociation changes results (bits). lccc today honors
strict FP semantics everywhere (the FP strict min/max folds of v5 are
the exactness discipline; the FMA contraction follows
`-ffp-contract=fast` GCC default at -O2 which we already match). The
nbody epic therefore has a HARD gate: `-fassociative-math` (implied by
`-ffast-math`), off by default, with the differential test contract
switching from bit-exact to ULP-tolerance (the benchmark oracle's
existing epsilon machinery).

### 4.2 The decomposition

Per body i, the j-loop accumulates `fx, fy, fz` and the pair distance:

```
dx  = x_i − x_j   (three components, unit stride over j)
r²  = dx² + dy² + dz²          (the §1.2 leaf — already optimal)
inv = 1/√(r²)  (vrsqrtps + one Newton step under fast-math)
f   = m_j · inv³               (vfmadd chains)
fx += f·dx  (fy, fz)           — 3 independent reductions
```

The vectorizable form processes 4 j-bodies per iteration (F64x4 /
2×F64x2 lanes): dx/dy/dz become 4-lane gathers (the affine-window
addressing of BB-SLP v4 generalizes to the loop's induction variable),
the reduction uses the classic **multiple accumulators + final
horizontal add** (4-wide fx/fy/fz vectors, `hadd`-tree at exit —
this is the reassociation the flag permits).

### 4.3 Why this is sequenced AFTER Epic A

* It needs `-fassociative-math` plumbing through the driver → pass
  config (a small, mechanical change, but a SEMANTIC contract addition
  that deserves its own gate + docs).
* Its correctness contract is tolerance-based — the differential oracle
  needs the ULP-comparison mode wired for FP outputs (exists for the
  benchmark oracle, not yet for the correctness runner).
* Its payoff (nbody-class HPC codes) is real but narrower than
  adler32's (every zlib consumer on the planet, including the kernel's
  own decompression path).

### 4.4 Expected result

The classic 4×-wide F64 reduction with FMA + vrsqrt approximation
matches icx `-ffast-math` on the 14700KF within the port budget
(2 FMA ports × 4-wide); the measured target is the benchmark suite's
existing nbody program — the oracle comparison becomes
`|lccc − gcc_fastmath| ≤ 4 ULP` per output.

---

## 5. Test and gating plan (both epics)

1. **adler32**: bit-exact differential vs the scalar reference over
   adversarial lengths (§3.4); a codegen gate pinning the packed shape
   (`vpsadbw` + `vpmaddubsw` + `vpmaddwd` + the vs1/vs2 ymm homes, no
   scalar chain left in the wide body); the 5552-bound peel test with
   `% BASE` boundary inputs.
2. **nbody**: tolerance differential (≤ 4 ULP vs the scalar loop at
   `-fno-associative-math`), plus a NEGATIVE test: without the flag,
   the loop must stay scalar-chain (bit-exact) — the gate that keeps
   the default honest.
3. Both wired into `ci_local.sh` as fast gates, mirroring the v6 gate.

---

## 6. Decision

Epic A is **designed and ready to implement** in the loop vectorizer
(exact mathematics, existing instruction families, existing homing and
fold infrastructure; the only new IR families are the madd pair, which
follows the W5 admission pattern). Epic B is **designed, gated on the
fast-math contract**, and sequenced after A. Neither touches BB-SLP's
reassociation-free charter — the charter stays exactly as proven.
