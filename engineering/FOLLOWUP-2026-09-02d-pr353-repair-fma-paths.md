# Follow-up 2026-09-02d — PR #353 merge repair + scalar FMA destination paths

Session base: `b4ba9c5` (PR #353 = general-cloner revival + hardening,
merged with one reviewer edit), then `d1dba8d` (PR #354).
Snapshots S13–S17.

## 1. Audit verdict on the #353 review claim

**Claim:** both cloners must check `init + trip*step` BEFORE rewriting
IR; in the pre-#353 source the checked bail sat after header/latch
retargeting, so an overflowing final IV returned `false` with the CFG
half-rewritten.  The reviewer moved the check as "the only edit".

**Verdict: the underlying point is VALID — my S10 patch did have the
bail after the mutations** (verified by red/green: with the bail moved
back, the new regression test fails with the latch retargeted).  But
**the merged implementation was broken three ways** — upstream main
`b4ba9c5` did not compile:

1. The hoisted checked block landed only in `try_complete_unroll_two_block`;
   in `try_complete_unroll_general` the `final_iv_n` *use* remained with
   its definition deleted → `E0425: cannot find value final_iv_n`.
2. The two-block cloner kept BOTH the hoisted check and the old
   post-mutation copy (shadowed, redundant, still after mutation).
3. The tests module gained a duplicated, spliced copy of
   `substitution_covers_twenty_five_non_algebraic_use_positions`
   containing the invalid token `Instruction::Atofeed the clone chain,`
   (parse errors at two brace sites).

Repair (S13): hoist completed in the general cloner (right after the
trip count, mirroring the two-block placement), redundant post-mutation
check dropped, corrupted duplicate test removed.  Audited the whole
bug class: after the first IR mutation, neither cloner contains any
`return false` (checked against every mutation pattern incl.
extend/insert/retain; the only helper calls between the guards and the
first mutation take `&IrFunction`).

New regression `general_cloner_overflow_bail_leaves_cfg_untouched`
drives the general cloner with init 0 / limit `i64::MAX` / step 2^62
(trip 2, final 2^63) and asserts `n == 0`, block count 7, latch
back-edge and header exit branch untouched.  An executable C regression
is impossible for this shape: any loop that trips the final-IV overflow
executes a signed-overflow increment when left rolled (verified — the
binary loops forever), so the cloner-level test is the right instrument.

## 2. dot8 follow-through: scalar FMA destination paths (S14/S15)

`dot8d` (stride-4 dot product, `-O3 -march=x86-64-v3`), lccc vs gcc:

| stage | lccc insns | note |
|---|---|---|
| start of session | 41 | unrolled+FMA already (c72ff2c), scratch-heavy |
| S14 | 37 | FMA scratch path takes lhs XMM home as src1 (kills `vmovsd home→xmm1`; xmm0 is never a home, so no alias; xmm2-staging exception preserved) |
| S15 | 33 | path 3: const acc + lhs/dest shared home → `vfmadd213sd %xmm0, %rhs_home, %dest_home` (dest = src1·dest + src2), result lands in the home; no store-back |
| S18 | 29 | text peephole `fold_fma_memory_src2`: single-use `[v]movs{d,s}` load folded into the adjacent FMA3-231 memory-src2 slot (function-wide last-mention liveness proof, call veto for xmm0-7); runtime at gcc parity (306-311ms both, 100M-iter driver) |
| gcc | 21 | folds b-loads into FMA memory operands; keeps zero in a register |

Caught during S15 validation: the first cut emitted the 213-form
operands in the wrong AT&T order, computing `0·lhs + rhs` (dot8d →
14.375 instead of 8.75; exactly reproducible from the miscompilation
algebra).  Fixed and pinned by the regression oracle.  Lesson re-applied:
battery before every snapshot, no exemptions.

Remaining dot8 gap, with designs:
* ~~**FMA memory src2 (231-form)**~~ — DONE (S18): the encoder form
  existed all along (`vfmadd231sd (%eax), %xmm2, %xmm0` is in the i686
  golden test); what was missing was the x86-64 peephole.  See the S18
  row above.
* **Resident zero** — 4×`xorpd %xmm0` re-materialise +0.0 per unrolled
  accumulator init; a cached "xmm0 holds +0.0" flag would kill 3, but
  xmm0 is written from 11 codegen files, so the invalidation surface is
  large.  Deferred deliberately.

## 3. Evidence

| check | result |
|---|---|
| upstream `b4ba9c5` compile | FAILS (parse errors + E0425) — recorded before repair |
| repaired tree compile | clean |
| unit tests | 1544/0 (new: `general_cloner_overflow_bail_leaves_cfg_untouched`) |
| red/green on the hoist | bail-after-mutation fails the new test (latch retargeted) |
| regression suite | 571/0/15 after every snapshot |
| benchmark outputs | 152/0/0 |
| `ir_verify_sweep.py` | clean |
| dot8d runtime vs gcc oracle | byte-identical outputs at every stage |

## 4. Session S17 — PR #354 red-team + i686 encoder differential audit

Session base: `d1dba8d` (PR #354 = i686 backend + #353 repair merge).

**#354 verdict.** C4/C5 VEX selection, bit layout, the FMA3 opcode grid,
and `needs_vex_ext` are SDM-exact; the I128 const fix verified
end-to-end. Two of my own earlier findings were RETRACTED: the alleged
W-bit bugs (gas 2.44 emits W=0/C5 for vsqrtpd/vcvtpd2ps/vaddpd — the
SDM "W1" is conservative; lccc matches gas) and the "no VEX sqrt"
claim (a truncated grep hid vsqrtps/vsqrtpd in the dispatch table).
Lesson recorded: never trust a truncated enumeration; gas is the oracle.

**Differential audit.** Built a mnemonic-level harness (lccc `-m32` vs
GNU as 2.44, every mnemonic of the i686 encoder × up to 20 operand
templates): **827 mnemonics swept — 195 VEX + 632 legacy byte-exact,
0 mismatches** at the end. Real bugs found and fixed (all pre-existing,
all now pinned by `legacy_sse_encodings_match_gas_golden_bytes`):

| bug | wrong bytes | correct |
|---|---|---|
| `extractps` emitted PEXTRW opcode, reg-arm operands swapped | `66 0f 3a 15` | `66 0f 3a 17 /r ib` |
| `insertps` emitted PEXTRB opcode | `66 0f 3a 14` | `66 0f 3a 21` |
| `cvttpd2dq` carried CVTPD2DQ prefix (rounding, not truncation) | `f2 0f e6` | `66 0f e6` |
| `mulw/divw/idivw` double operand-size prefix | `66 66 f7 /n` | `66 f7 /n` |
| bare `iret` selected IRET16 | `66 cf` | `cf` |
| `nop mem` dropped its operand (bare 90) | `90` | `0f 1f /0` |
| `popw m16`, `pextrw m16` memory forms missing | reject | `66 8f /0`, `66 0f 3a 15` |

VEX additions from the same sweep: `vpshufd` re-wired from the 0F3A
map to its true 0F-map form (`c5 f9 70`), `vpshufhw`/`vpshuflw`
(pp2/pp3) and 3-operand `vsqrtss`/`vsqrtsd` added (gas rejects the
2-operand scalar forms). Legit rejects left as rejects: EVEX-only
shapes (vpermpd/vpermq reg-form, vmovd xmm,xmm) and system insns
(lldt/ltr/str/sldt) — out of scope for this i686 target, kernel boot
gate SKIPs in this environment anyway.

**Merge-AI nit fixed:** `clz_ctz_popcount_target_isa.c` word
`0xfffffffffffffffeul` — on i686 `unsigned long` is 32-bit, so the
literal silently truncated to `0xfffffffe` and the 64-bit path never
saw the intended pattern. `ull` now; A/B vs gcc matches
(`951cafda3d84ba90` both sides).

**S17 evidence:** unit 1545/0 (2 golden tests), regression 571/0/15
(AB-diff failures 0), benchmark outputs 152/0/0, differential audit
827/827 byte-exact vs gas 2.44.

## 5. Backlog (updated)

1. Typed `Call` lowering in MachInst isel (largest silent-reject bucket).
2. Float layer gate audit: `can_lower` currently rejects ALL float
   forms (`isel.rs:1020-1022`); the FMov/FAlu kill-switch layer
   (1<<15/1<<16) is not reachable through this gate — reconcile before
   extending float coverage (Store(float) rejects visible even in a
   5-instruction loop, e.g. `mx()` max-reduction).
3. Vectorizer: global-array map-loop bailout.
4. Memcpy(92)/Store-other(79) typed lowering.
5. Loop-gate `loop_insts ≤ 32` raise experiment.
6. Linker-oracle script for LTO/whole-program comparisons.

## 6. Session S18 — FMA memory-src2 peephole (dot8 33 → 29)

New text peephole `fold_fma_memory_src2` (x86-64, `memory_fold.rs`):
folds a scalar FP load whose register is never mentioned again into the
memory-src2 slot of an adjacent FMA3-231 instruction.  Liveness proof is
function-wide (exactly two mentions of the loaded register from the load
to `.cfi_endproc`, token-bounded so `%xmm1` ≠ `%xmm11`), with a call
veto for `%xmm0`-`%xmm7` (implicit variadic-argument reads).  Stronger
than the block-local scan of the older commutative-op fold, so reused
homes (`%xmm5` in two loop iterations) fold for the last def only.
10 unit tests (fold shapes + every veto).  `CCC_PEEPHOLE_SKIP=fma_mem_fold`
bisection hook.

**S18 evidence:** unit 1556/0, regression 571/0/15 (AB-diffs 0),
benchmark outputs 152/0/0, dot8d 29 insns (gcc 21) runtime at parity
(306-311ms both sides, 100M-iteration driver, outputs identical).

## 7. Session S19/S20 — #355 red-team + Godbolt-oracle FMA shaping

Base moved to `4280c6c` (PR #355: i686 dead-frame elision + slot
forwarding).  Red-team verdict: frame elision guards sound (and CFI-
safe: i686 emits no per-adjust `.cfi_def_cfa_offset` yet, see
prologue.rs TODO — revisit when that lands); both forwarding
extensions miscompile-class wrong.  Repairs (S19, commit e2d623d):
width-mismatched forwarding refused (a `movl slot` reload after a
narrower store reads unwritten garbage bytes, NOT zeros — kept only
the zext-terminated fusion, which is provable); same-width W/B
rewrites keep the reload's narrow destination (the pre-existing
same-slot arm had been widening `movw slot,%dx` to `movl %s32,%d32`,
and #355's movzwl arm behind it was unreachable dead code);
`narrow_reg_name` no longer advertises the 64-bit-only %spl/%bpl/
%sil/%dil encodings; i686 now emits `#APP`/`#NO_APP` around inline-asm
templates and the peephole pins the regions (`LineKind::InlineAsm`:
no rewrite, window barrier, reads-everything in censuses, veto in
both frame passes) — ported from the x86-64 mechanism whose absence
made every i686 text pass free to rewrite user asm.

S20 (Godbolt/gcc-oracle FMA shaping, x86-64): gcc's dot8 strategy
decoded — ONE resident zero, first-iteration accumulators via
`vfmadd132sd MEM, %xmmZERO, %dstA` (dst preloaded with `a`, `b`
folded into the memory SRC3 slot, zero as vvvv addend).  Two new
sound text passes reproduce it: `fold_zero_addend_fma213_to_132`
(triple: b-load + xorpd + 213-form → xorpd + 132-form with the load
in the memory slot; last-reader proof — the first post-FMA mention of
the multiplier must be a pure full-width overwrite or absent) and
`eliminate_redundant_xmm0_zeroing` (block-local zero-liveness state
machine: reads preserve, last-operand writes/calls/labels/opaque asm
kill).  dot8d 29 → 22 insns (gcc 21; the remaining movsd is return-
placement regalloc), runtime at parity (305ms both, 100M-iter
driver), outputs identical.  9 unit tests.  Kill switch:
`CCC_PEEPHOLE_SKIP=fma132_zero`.

**S19/S20 evidence:** unit 1582/0, regression 571/0/15 (AB-diffs 0),
benchmark outputs 152/0/0, differential audit vs gas 2.44 827/827
byte-exact on the rebased tree, i686 `#APP` round-trip verified
through the integrated assembler.
