# SSE2 codegen: the per-instruction matrix, the aggregate round trip, and what is left

Date: 2026-10-21. Base: `771f8a3` (`main`) + this branch. Oracles: **GCC 16.2.0**
and **Clang 23.1.3**, both restored from the repository's tag cache
(`harness-cache-1`, `harness-cache-4`) and verified against its SHA-256
manifests; LCCC built with the cached **Rust 1.97.0** MSRV
(`rust-cache-1.97.0`). Nothing in this document is an estimate: every number is
printed by the harness below, and the commands are at the end.

## Why a per-instruction matrix

`include/xmmintrin.h` and `include/emmintrin.h` give LCCC the whole SSE/SSE2
intrinsic surface (394 entry points). The existing gates answer adjacent
questions -- the regression corpus checks *results*, `check_vectorize_isa_gate.sh`
checks which vectorizer arm runs, the asm census counts mnemonics corpus-wide --
but none of them compares, **per instruction**, how much code LCCC needs against
GCC and Clang for the same source. A kernel can be bit-exact and still be three
times the instruction count of both oracles, and that is exactly what a
memory-backed intrinsic ABI produces.

`tests/regression/check_sse2_codegen_matrix.py` is the instrument. It is built
from a declarative table with **one row per SSE/SSE2 instruction** (122 rows:
packed/scalar FP, integer SIMD, shifts, shuffles, packs, conversions, moves,
plus a loop-nest row), and for each row and compiler it:

1. compiles the row's kernel at `-O2` for the plain `x86-64` baseline,
2. extracts that kernel's instruction stream from the emitted assembly
   (tolerant of LCCC's 4-space indentation, GCC's tabs, Clang's `#` comments and
   every compiler's internal labels -- an extractor that stopped at the first
   `.LBB` label silently measured only one basic block),
3. censuses instructions per mnemonic,
4. runs the kernel and hashes the raw output bytes, and
5. in `--mode timing` measures CPU time per call.

Three facts fail independently: **lowering fidelity** (the row's canonical
mnemonic appears), **differential correctness** (LCCC's output hash equals
*both* oracles' bit for bit), and **instruction economy** (LCCC's count against
the better oracle). The committed `tests/regression/sse2_codegen_matrix.json`
records the verdicts, the instruction-count debt and the mnemonic sets. The gate
ratchets on the quantity this repository actually controls: **LCCC's own
instruction count may shrink, never grow**, and a mnemonic that was emitted may
not disappear. The *oracle gap* (and the win/tie/loss verdict it implies) is
reported but enforced only under `--strict-oracle-debt`, because the oracle is a
per-host BUILD: a distribution's GCC 16.2.0 and the upstream tarball's 16.2.0
differ by a few instructions, and once LCCC is near parity (the median row is
now +2 instructions) a one-instruction oracle difference would fail an innocent
change on a different runner. `--self-test` perturbs the baseline in memory and
asserts the checker notices, so the ratchet cannot silently rot.

## Root cause: every `__m128*` is a 16-byte aggregate carried as `I128`

LCCC has no IR vector type. `__m128d`/`__m128`/`__m128i` are structs of two
doubles in `include/lcccsimd.h`, and an aggregate *value* that crosses a call
boundary in a register pair travels as `I128`
(`src/ir/lowering/aggregate_abi.rs`: "Returned in a register pair (9..=16
bytes), packed as I128 in the IR"). Every SSE intrinsic wrapper is
`__always_inline__`, so the packing written for the call boundary survives
inlining together with its unpacking:

```text
%lo  = load %p           (I64)
%hi  = load %p+8         (I64)
%clo = cast %lo -> I128
%chi = cast %hi -> I128
%sh  = shl %chi, 64
%agg = or  %sh, %clo          <- the 16-byte value, in two GPRs
%dlo = cast %agg -> I64 ; store %q
%shr = lshr %agg, 64
%dhi = cast %shr -> I64 ; store %q+8
```

The net effect is a 16-byte copy, and it cost eight integer instructions, two
GPRs and two stack homes -- for *every* intrinsic, on top of the operation
itself. `_mm_loadu_pd` alone compiled to 40 instructions against GCC's 5.

## The fix: `aggregate_pair_roundtrip` (`CCC_DISABLE_PASSES=aggcopy16`)

`src/passes/aggregate_pair_roundtrip.rs` recognises that exact shape in one
basic block and replaces it with the **intrinsic 128-bit copy pair**
(`Loaddqu`), not a `Memcpy`: `Loaddqu`/`Storedqu` are what
`vector_temp_promotion::fuse_vector_loads` already knows how to forward, so the
copy then disappears into the consumer's memory operand instead of being
emitted at all.

The match is deliberately narrow and fail-closed: one basic block; every
intermediate single-use and single-definition (multi-def webs are refused
outright, the `load_first`/`gvn` guard); the two halves provably 8 bytes apart on
both sides; no intervening memory write that could alias either endpoint; and a
provenance-based disjointness proof between source and destination (distinct
stack homes, or a stack home against a pointer this frame cannot name, or a
same-root distance of at least 16 bytes) so a partially overlapping copy is
never rewritten. Seven unit tests cover the accept cases and each refusal.

### Measured effect (same tree, pass on vs off)

| | rows improved | total instructions (122 rows) | median ratio vs best oracle | geomean | median delta |
|---|---|---|---|---|---|
| pass off | -- | 6 782 | 2.96x | 3.40x | +21.5 |
| pass on | **59 / 122** | **3 677** (-45.8 %) | **2.00x** | **2.26x** | **+10.0** |

Largest single-row effects: `divpd`/`divps`/`maxpd`/`minpd`/`cmpgtpd`/`cmpgtps`
113 -> 39 instructions (2.90x); the whole integer SIMD block (`paddb` ... `paddq`,
`pand`, `pxor`, `pack*`, `punpck*`, `pmul*`, `psadbw`) 65 -> 17 per row.

All 122 rows still hash bit-exactly against **both** oracles after the change,
and the emitted-mnemonic sets are unchanged or improved.

## What the matrix says is left (the recorded debt)

The matrix is honest about the remaining gap. As of this change, every row is
still behind the better oracle, and the committed baseline records the exact
debt per row. The findings, in descending order of value (step 1 is closed; the
rest are open):

1. ~~**Stack-home round trips inside the intrinsic chain.**~~ **DONE** — see
   "Step 2" below. `_mm_add_pd` 35 -> 31, `pshufd` 18 -> 14 (its frame
   disappears entirely), 118 of 122 rows improve, no row regresses.
2. **Bitwise FP ops run in the integer domain.** `_mm_and_pd`/`_mm_or_pd`/
   `_mm_xor_pd`/`_mm_andnot_pd` and their `ps` twins emit `pand`/`por`/`pxor`/
   `pandn` instead of `andpd`/`orpd`/`xorpd`/`andnpd`. Functionally identical,
   but on Intel the integer-domain form on FP data pays a bypass delay. 9 rows.
3. **Scalar `max`/`min` are branches, not `maxsd`/`minsd`.** `_mm_max_sd`/
   `_mm_min_sd` lower as a compare plus a select (102/103 instructions against
   GCC's 12). `a > b ? a : b` *is* `MAXSD`'s semantics (including the NaN case,
   which returns the second source), so this is a pure pattern-match
   opportunity in the backend's select-to-instruction path -- *after* the
   GPR-pack disease of step 3 is gone, because at 102 instructions the select
   is not what makes the row slow.
4. **`movapd`/`movdqa` are never emitted for a 16-byte copy**; LCCC uses
   `movdqu`. For a `void *` source that is correct (no alignment fact), so this
   row's expectation is idealised rather than a defect: the matrix's own
   `movdqu`-vs-`movapd` distinction is recorded, not failed.
5. ~~**`_mm_cvttsd_si32` does not emit `cvttsd2si`.**~~ **FIXED** — and it was
   worse than a mnemonic finding. The header routed *both* truncating
   conversions through the rounding forms:

   ```c
   _mm_cvttsd_si32(a) -> cvtsd2si    /* rounds per MXCSR.RC */
   _mm_cvttss_si32(a) -> cvtss2si
   ```

   Under the default round-to-nearest mode `2.7` converted to **3**, where the
   intrinsic's contract (and GCC, and Clang) give **2**; under
   `MXCSR.RC = round-up` it is wrong for every non-integral input. Nothing in
   the corpus exercised a non-integral conversion, so the row's hash check
   passed — the matrix recorded it as a *fidelity* finding until the
   differential test below was written.

   Fixed by giving truncation its own intrinsic ops (`CvttSs2Si128`,
   `CvttSd2Si128`), emitted as `cvttss2si`/`cvttsd2si` on x86-64 and i686, and
   wired through `scripts/gen_lcccsimd.py` so the header cannot drift back.
   `tests/regression/check_sse2_truncating_convert.sh` proves it three ways:
   the emitted mnemonic, execution under all four MXCSR rounding modes, and a
   differential run against GCC 16.2 — and it fails on the previous headers
   with the exact per-input failures.

   The 64-bit forms of the same family were wrong in the *other* direction and
   are fixed too: `_mm_cvtsd_si64` computed `d >= 0 ? d + 0.5 : d - 0.5`
   (round-half-away-from-zero, not MXCSR's mode; wrong at and above 2^52,
   where the addend is absorbed; four instructions instead of one) and
   `_mm_cvtss_si64` was a plain truncating cast (that is
   `_mm_cvttss_si64`'s contract). Both now lower to `cvtsd2si`/`cvtss2si`
   with a 64-bit destination through `__builtin_ia32_cvtsd2si64` /
   `__builtin_ia32_cvtss2si64`, which are new IR intrinsics
   (`Cvtsd2Si64`, `Cvtss2Si64`) so the lowering is architecture-aware rather
   than a header hack. `2.5 -> 2`, `-2.5 -> -2`, `2^52 + 0.5 -> 2^52`, all
   bit-identical to GCC 16.2 and pinned by the same gate.

## Step 2: 16-byte XMM slot forwarding (`store_forwarding`, `CCC_NO_XMM_SF`)

The peephole already forwarded XMM slot stores into their reloads, but only for
the scalar widths: `MoveSize` had no 16-byte member, so `movdqu`/`movups`/
`movdqa`/`movapd` slot moves were classified as opaque `Other` lines with a
*point* (8-byte-credited) offset instead of a 16-byte slot access. Two
consequences, both measured: the reloads were never forwarded, and every
aliasing question about a 16-byte access was answered from an 8-byte footprint.

`MoveSize::DQ` (16 bytes) is now a first-class slot width. The parser accepts the
six 16-byte slot mnemonics plus their VEX spellings; the slot mapping machinery
(`invalidate_slots_at`, `SlotEntry`, `RegHomes`) is range-based and needed no
change; `dead_code`'s never-read-store elimination and the frame compactor see
the real extent for free.

The forwarding rule is stated in terms of the register forms' *architectural*
semantics, because that is where the bug class lives:

* an XMM home serves any load that reads **at most** the bytes the store wrote,
  at the same offset, provided the replacement zeroes every bit the memory form
  zeroes — true for `movq` (SD/Q: copies 0..63, clears 64..127) and for the
  16-byte forms (copies all 128);
* the 4-byte forms are **not** eligible unless the VEX `vinsertps` sequence is
  used, because `movss %xmmA, %xmmB` *merges* bits 32..127 where a memory load
  zeroes them. This asymmetry is exactly what the pre-existing test
  `fp_store_fp_load_same_register_keeps_zeroing` pins;
* a full-width self reload (`movdqu slot, %xmmN` into the register that wrote
  it) is a **no-op** and is deleted. At any narrower width it is not: the load
  still has to zero the bits above its own width, which is why the same-register
  rule is gated on `MoveSize::DQ` rather than applied to every width;
* the rewrite keeps the instruction's **own mnemonic**. An aligned `movdqa`/
  `movapd` store stays aligned (the alignment promise is about the address, not
  about the source register) and a VEX form is not demoted to its SSE spelling
  — the relay emits `movdqa %xmm6, 40(%rsp)`, never `movdqu`.

Nine unit tests cover the accept paths and each refusal: clobbered backing
register, partially overlapping store, disjoint adjacent store (must still
forward), the relay pair, mnemonic preservation, and the classification extents
(including the negative case: `movhps`/`movlpd` are half-width moves and must
*not* join the 16-byte class).

### Measured effect (same tree, before vs after)

| | rows improved | total instructions (122 rows) | median ratio vs best oracle | geomean |
|---|---|---|---|---|
| after step 1 | -- | 3 677 | 2.00x | 2.26x |
| after step 2 | **118 / 122** | **3 295** (-10.4 %) | **1.65x** | **1.94x** |

No row regressed; all 122 rows still hash bit-exactly against both oracles.
Cumulatively (before step 1 -> after step 2): 6 782 -> 3 295 instructions
(-51.4 %), median ratio 2.96x -> 1.65x.

Four scalar rows (`addsd`, `subsd`, `mulsd`, `divsd`) *lose* the `movsd`
mnemonic. That is the improvement, not a regression: the aggregate's slot round
trip is 16 bytes wide, and the compiler now moves it as one `movdqu` instead of
an 8-byte `movsd` that covered half of it. The baseline records the new sets;
the rows' signature mnemonic (`addsd` etc.) is still emitted and still checked.

## Step 3: the remaining disease — scalar-pair intrinsics run through GPRs

`_mm_add_sd(x, y)` is plain C in `include/emmintrin.h`
(`__a.__val[0] += __b.__val[0]; return __a;`), and `__m128d` is a 16-byte
aggregate carried as `I128`. After inlining, the IR for one call is

```text
%y0 = load %y ; %y1 = load %y+8 ; %yagg = or (cast %y1 << 64), cast %y0
%x  = Loaddqu %xp
%res = fadd (f64)load %x, (f64)(cast %yagg -> I64)
%ragg = or (cast %res << 64), cast (load %x+8)
store two GPR halves of %ragg ; Storedqu -> out
```

The measured cost is `addsd` 80 instructions against GCC's 12, and
`maxsd`/`minsd` 102/103 against 12 -- the largest absolute gap in the matrix,
and the reason the scalar rows are the last ones still far from parity.

Two independent fixes are possible and both are real:

1. **Local, provable, and measured to be harmful on its own.** Folding
   `Cast(Or(Shl(hi, k), lo), 128 -> w)` to `Cast(lo)` (`k >= 64`, `w <= 64`) is
   a bit-level identity, so it was implemented and measured: `addpd` went
   **35 -> 102 instructions**, because the pack shape is exactly what
   `vector_temp_promotion`'s whole-aggregate promotion matches, and breaking it
   locally destroys the global rewrite. It was reverted. *Any future attempt
   must promote the aggregate first and fold second, and must be measured on
   the matrix before it is believed.*

   The refined version -- fire only when the packed value has **exactly one
   reader**, so the high half is provably dead rather than merely unobserved --
   was also implemented and measured: it changes nothing (122/122 rows
   identical, `addsd` still 80 instructions, `shlq`/`orq` still emitted),
   because the simplify pass runs without use counts on this path. It was
   reverted too. The rule is sound and worth reviving *with* a real use count
   plumbed through `simplify::run_with_config`, but shipping a fold that
   provably never fires is technical debt, not an optimization.
2. **The architecturally right fix**: give the aggregate-valued intrinsic
   operands a register home. `vec_ssa`/`vector_temp_promotion` already do this
   for the `Vec*` intrinsic family (`collect_sse128_chain_values`,
   `allocate_vector_registers_destructive`); the scalar-pair family needs the
   same treatment, which would let `addsd`/`maxsd` be one XMM instruction
   instead of a GPR pack/unpack around it.

## Reproducing

```bash
# Oracles from the tag cache (SHA-256 verified against their manifests):
git fetch origin tag harness-cache-1 tag harness-cache-4 tag rust-cache-1.97.0
#   harness-cache-1: gcc-16-bookworm-x86_64.tar.xz, binutils-2.47.tar.xz
#   harness-cache-4: clang-23.1.3-linux-x64.tar.xz
#   rust-cache-1.97.0: rustc/rust-std/cargo/clippy/rustfmt 1.97.0

CCC=target/fastbuild/lccc GCC=gcc-16 CLANG=clang \
    python3 tests/regression/check_sse2_codegen_matrix.py --report
CCC=target/fastbuild/lccc GCC=gcc-16 CLANG=clang \
    python3 tests/regression/check_sse2_codegen_matrix.py --mode timing --only matmul_ikj --report
CCC=target/fastbuild/lccc GCC=gcc-16 CLANG=clang \
    python3 tests/regression/check_sse2_codegen_matrix.py --check          # the gate
CCC_DISABLE_PASSES=aggcopy16 CCC=target/fastbuild/lccc GCC=gcc-16 CLANG=clang \
    python3 tests/regression/check_sse2_codegen_matrix.py --check          # must FAIL
```

The last command is the gate's own regression test: with the pass disabled, 59
rows grow, and `--check` reports exactly those 59 debts. It passes only with the
pass on.
