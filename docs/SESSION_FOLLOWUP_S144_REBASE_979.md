# Session follow-up: rebase onto #979, red-team of #972/#979, ABI and dispatch fixes (ms178-1)

**Base:** `5cca27779` (ms178/lccc main, Merge #984), on `c5722a4d7` (#981),
`2031eddd7` (#982), `69e2dc4df` (#980), `867e83a9a` (#979) and `33d948722`
(#972).
**Deliverable:** `ms178-1.patch` (`git format-patch origin/main..HEAD`).
**Oracles:** GCC 16.2.0, Clang 23.1.4 (official release, sha256-checked), ICX
2026.2, GNU as/ld 2.47, gdb 16.3, Valgrind/Callgrind. The VM has no PMU, so
every runtime figure below is a Callgrind instruction count (Ir) unless it is
marked as a correctness result.

## Rebase onto #984 (`5cca27779`): conflicts and verdicts

#984 (`351f2f46e`: half-float/bfloat16, hardening, UBSan trap, whole-program
IPA, lccc-ld parity; 138 files) conflicted in four places, all resolved by
keeping both sides: `backend/mod.rs` (upstream's `stack_clash_protection` and
`stack_protector_guard` fields beside this series' i686 FP options),
`ir/intrinsics.rs` (upstream's `F16ToF32`/`F32ToF16` beside the
`VecWidenAddI32x4ToI64x4` rename), the x86 prologue (upstream's
`emit_fpo_stack_alloc_x86` beside the splat collectors) and the driver
contract script (upstream's stack-clash block beside the i686 trapv
matrix). The files #984 changed without a textual conflict (`expr.rs`,
`expr_access.rs`, `expr_types.rs`, `stmt_return.rs`, `trapv.rs`) were checked
by the build and the gates below, not by inspection alone.

**Agree.**
* Half-float/bfloat16 through the soft conversion paths, and the variadic
  rule (the value travels in the low 16 bits of a `float` slot). A
  differential over arithmetic, double rounding (`1.000488281250001`),
  overflow to infinity, subnormals, int/long long/unsigned long long
  conversions, NaN compares, `__builtin_fabsf16` and a variadic `_Float16`
  sum gives `a7f75dc754af3021` for lccc at -O0, -O2 and -m32 -msse2, the same
  as GCC 16.2 (all three) and Clang 23.1.4.
* UBSan trap mode (`-fsanitize=undefined -fsanitize-trap=undefined`): 51
  regression fixtures that include the x86 intrinsic headers behave the same
  with and without it once the header defect of item 12 is fixed. Running
  them is how item 12 and a signed lane overflow in this series' own
  `vector_small_packed_wrappers.c` (GCC's UBSan traps on it too) were found.

**Found by the rebase validation (fixed in this series).** The MMX
differential crashed in `t_lanes` (s169 and s170): once `__m64` is a vector,
`_mm_extract_pi16`'s macro casts its operand to `__m64`, and the
vector-to-vector cast returned a packed call result unchanged as the cast's
"address". See item 7.

## Rebase onto #980/#982 (`2031eddd7`): verdicts and fixes

Conflicts: the `pipeline.rs` instruction scanners (upstream's
`first_instruction_if` now passes the function name; every caller was
re-checked, not only the conflicting hunks) and the end of x86 `emit.rs`
(both test modules kept).

**Agree.**
* `0862669ee` SSE1 bit-carrier bridges: `%rsp`-balanced, flag-preserving,
  CFA-adjusted under FPO. A red-zone clobber was hypothesised and refuted
  (the bridge moves `%rsp` before it stores).
* `f438d56b6`: a VEX.128 `vpxor` zeroes the whole YMM on AVX1; the old
  VEX.256 form needed AVX2 (a real bug, correctly fixed) and not arming
  `dirty_upper_ymm` for a zero idiom is right.
* `e864cebe1`: permission follows the ISA of the instruction actually
  emitted; native packed add/sub/mul stay in typed Vec SSA.
* #982 `engineering/isa/sse-avx-public-api.json` (216 KB) is machine-read
  input to `audit_sse_avx_public_api.py`, so its size is justified.

**Disagree / defects found.**
* #980 left raw SSE2 moves on SSE1 scalar routes that only exist with
  register homes. lccc refused valid float C under x86-64 `-mno-sse2` at
  -O2/-O3 (360 of 480 differential runs) while -O0 was clean: XMM<->GPR
  `movd`/`movq` in `operand_to_rcx` (float select), call-result moves,
  `value_to_reg`, float_ops/i128 libcall results and mixed copies; F32
  copies loaded with 8-byte `movsd`/`movapd`; the call-argument hazard
  spill. Fixed (`x86: route remaining SSE1 scalar XMM moves through the
  bridge`).
* `check_legacy_sse_permissions.py`'s SSE1 whitelist lacked
  `cvt{,t}ss2si{,l,q}` and `cmpss`/`cmpps` (SSE1 float->int and compare),
  so legal programs would fail it; the compiler's own fence
  (`first_sse2_reference_unless`) already allowed them. Fixed, with new
  probes; each fails on the binary before its fix.
* #982 `engineering/evidence/.../corpus-screen.json` (133 KB) is evidence
  over the 100 KB policy; recorded, not changed (upstream's file).
* **Retracted:** an earlier note in this session called
  `check_legacy_sse_permissions.py` a dead gate. It is not:
  `check_intrinsic_feature_gate.sh` runs it, in both CI mirrors. No new CI
  step was added; the new probes ride the existing gate.

**Own defect exposed by #980.** The i686 ISA floor (`4295a3151`) was
TU-wide and refused SSE2 in a `target("sse2")` function under
`-m32 -mno-sse2`, which GCC accepts and #980's gate pins. Fixed: a
function's own grant lifts the floor for that function only (`i686:
per-function target() grants lift the SSE floor for that function`).

**Pre-existing upstream wrong-code bug (x86-64 default ISA).**
`fp_copy_web_groups` could seed the XMM scan with a call-spanning
union-find root as web leader. Its interval was the envelope of the
eligible members, which does not cover its own call, so it was homed in a
caller-saved XMM and the call clobbered it. Pristine main `2031eddd7`
prints `b7162bc0` for `tests/regression/fp_copy_web_call_leader.c` at -O1
and above; GCC 16.2 and lccc -O0 print `b19e89fd`. The default-mode fuzz
found 5 divergences in 960 runs on main (seeds 197, 203) and 0 after the
fix (`regalloc: an FP copy-web leader must be scan-eligible`). Webs whose
root was eligible allocate byte-identically: across all 1081 corpus
sources at -O2 the assembly changes in exactly two files, the new fixture
and `fp_liveness_spill_relay_gap_computes_on_xmm0.c`. The latter was
correct before only because an explicit `%xmm2` save/restore happened to
bracket the call; it now spills the call-spanning value to its own slot,
one instruction shorter on the hot path.

**Fuzz false alarm, root-caused.** Seed 60 diverged at -O2. It converts
-5.5e9 to `int` (UB, C11 6.3.1.4p1); `-fsanitize=undefined` does not
include `float-cast-overflow`. The generator now guards the conversion,
and every program is screened with `float-cast-overflow` before it counts.

**Differential results** (`scripts/fuzz_float_scalar_vs_gcc.py`, 240
seeds, UB-screened): `--mode sse1` (-O0/-O2/-O2 -fomit-frame-pointer/-O3 vs
`gcc-16 -O2 -mno-sse2`) 960/960, 0 refused; `--mode default`
(-O1/-O2/-O3/-O2 -mavx2 vs `gcc-16 -O2`) 960/960.

**No performance cost.** Callgrind Ir is identical for s150 (pre-#980),
s151 (rebased) and the fixed tree on `bench.c`, `benchr.c` and `tail.c` at
-O2 and -O2 -mavx2 (for example 107295064 and 42573064 for `bench.c`), and
the s151/fixed assembly is byte-identical on all six.

## SSE2-baseline FP selects and select CSE (new this session)

Found by a census, not by guessing: at the SSE2 baseline (-O2, no
`-march`), 13 benchmark-corpus functions and 25 regression-corpus
functions selected FP values through a GPR `cmov` (XMM -> GPR, `cmov`,
GPR -> XMM). At `-O3 -march=x86-64-v3` there were none (VEX blends).

* **Legacy-SSE mask select** (`comparison.rs`
  `try_emit_fp_select_sse_mask`): a `cmpss`/`cmpsd` mask and
  `((T ^ F) & mask) ^ F` built in the destination home. That is LLVM X86
  `LowerSELECT`'s no-SSE4.1 shape. Branch lowering was rejected: it only
  wins when perfectly predicted (llvm-mca 27.0 vs 29.0 cycles on
  alderlake), and both arms are already speculated, so a mispredict
  would buy nothing.
* **GVN numbers `Select`.** The pass had no `Select` key, so a helper
  inlined three times (glibc's `rte` behind rint/nearbyint/roundeven)
  kept three identical selects. Their shared compare then had three uses,
  was materialised as a boolean, and pushed every select back onto `cmov`.

`libm_round_family` (registered workload, -O2, identical output):

| Build | Callgrind Ir | llvm-mca alderlake | llvm-mca znver4 | loop insns |
|---|---|---|---|---|
| before | 17.38M | 25.09 cyc/iter | 32.73 cyc/iter | 83 |
| mask select only | 18.41M | 20.02 | 24.82 | 88 |
| mask select + GVN | **11.03M** | **11.54** | **13.07** | 52 |
| GCC 16.2 | 21.75M | | | |
| Clang 23.1.4 | 29.02M | | | |

The middle row is why Ir alone is not the judge here: it counts the
rename-eliminated `movapd` copies, not the three ~3-cycle domain
crossings per select that are gone. (GCC's and Clang's rows have no mca
figure: their rounding helpers are out-of-line calls that mca cannot
model.)

**Whole-corpus effect (s158 -> s161, `artifacts/s81/cgsweep.py`, output
checked against GCC 16.2 on every workload):**

* -O2, 56 registered workloads: 55 assembly-identical; `libm_round_family`
  at the default build size 240,667 -> 208,924 Ir (-13.19%; GCC 16.2
  261,010). No workload regressed.
* -O3 -march=x86-64-v3: 56/56 identical (VEX blends already covered it).
* Regression corpus at -O2: 1082 files compared, 20 differ, **0
  with more XMM<->GPR crossings**. Crossings: `fp_select_sse_mask`
  374 -> 182; `fp_domain_crossing`, `ifconv_speculation_adversarial`,
  `vex_promote_semantic` 6 -> 0; `strict_recip_vex_pack_stress` 8 -> 2;
  `bb_slp_v6` 21 -> 15. The select CSE also shrinks integer code:
  `bool_thread_merges` 497 -> 309 lines, `cmp_replay_far` 251 -> 233,
  `vec_*_lane_map` and `vec_byte_runtime_invariant` -3..6%.

**Fixture** `tests/regression/fp_select_sse_mask.c`: 12 `SEL`
instantiations x 7 shapes (Eq/Ne/Lt/Le/Gt/Ge, +0 arms on either side)
over 12 special inputs (+-0, +-inf, NaN, denormals, equal pairs). The
result hash is order-canonical (NaN payloads depend on operand order) and
equals GCC's `b7ca90696c14c6cf 1728` at -O0..-O3, -fomit-frame-pointer and
-mavx2; `cmov` count 80 -> 0. Mutation check: swapping the `le`/`lt`
immediates, swapping the +0 shortcut's `and`/`andn`, or
mapping `ne` to `eq` each changes the hash, so the fixture can see every
predicate and arm-order error class. `-0.0` arms deliberately take the
generic path (an `and` with the mask would produce +0).

**Integer-condition FP selects are kept on `test`/`cmov`/`movq`.** For
`c ? x : y` with an integer `c` and FP arms, the current lowering costs
about 5 cycles on the condition path and 7 on the arm path (three
domain crossings). The mask alternative (`setne`, `neg`, `movq` to build
an all-ones lane, then `and`/`andn`/`or`) cuts the arm path to about 3
but lengthens the condition path to about 8 (llvm-mca, alderlake and
znver4). Neither dominates: which one wins depends on whether the
condition or the data arrives last, which only a PMU-backed profile on
the target can tell, and this VM has none. GCC and Clang branch here,
which is only good when the condition is predictable. Changing it
without that data would be guesswork, so the lowering is unchanged.

## Verdicts on upstream #981 (`c5722a4d7`: ring identities, pow2 dividers)

The series rebased onto #981 without conflicts. Its new fast gate
(`check_llvm_ring_identities.py`) passes against this series: 32 folded
forms, 1 survivor, 4 documented promotion boundaries. `fuzz_identities.py`
passes 100/100 programs.

**AGREE:**

* `bit_idioms_identities.rs`: every rule checked by hand is an exact ring
  or lattice identity, including at `C = INT_MIN`. Examples:
  `(a|b|c) ? a : 0 -> a` and `x * (x != 0) -> x`. Signed-remainder rules
  are limited to signed types, which matters because lccc lowers `SRem` to
  `idiv`. The kill switch `CCC_NO_RING_IDENTITIES` is useful for bisecting.
  Its `Select` outputs now also benefit from this series' GVN `Select`
  numbering.
* Power-of-two `SRem` as `((x + b) & m) - b` with `b = (x >> (w-1)) & m`:
  exact for every `x`, including `x = MIN`. Four ALU operations, against
  GCC's five (`sar`/`shr`/`lea`/`and`/`sub`).
* `dbbf2b735` (the `constant_fold` `signed_operand` guard, `!(8..=64)`):
  sound. A 64-bit width is the zero-shift case and must be included.

**DISAGREE / fixed here:**

* `scripts/fuzz_identities.py` hard-coded the author's paths, crashed on
  the first mismatch, and had no `--help`. Fixed in this series: argparse,
  `--lccc`/`--gcc` (defaulting to `CCC`/`GCC`), and reproducers kept under
  `research/`. It stays a manual tool, not a CI gate (no new CI wiring).
* `engineering/llvm-batch3-review.json` (179 KB) is not read by any script
  and is over the 100 KB evidence policy. This is recorded, not changed:
  upstream files are left untouched.
* A missed fold next to #981's dividers: a division or remainder by the
  type's minimum (`x / INT_MIN`, `x % LLONG_MIN`) still emitted `idiv`
  (10-18 cycles, not pipelined). It is now `x == MIN` for the quotient and
  `x == MIN ? 0 : x` for the remainder (`div_by_const.rs`
  `expand_by_signed_type_min`). For the quotient this is GCC 16.2's
  `cmp`/`sete`; for the remainder it is `cmp`/`cmov`. llvm-mca
  (cycles/iteration, alderlake/znver4) for `x % INT_MIN`: lccc 2.59/1.88,
  Clang 23.1.4 and ICX 2026.2 2.89/2.54
  (`lea`/`test`/`cmovns`/`and`/`add`), GCC 16.2 6.04/2.08 (seven
  instructions), s162 6.38/24.04 (`idiv`). `x / INT_MIN` ties all three
  oracles at 2.04/1.07. Covered by a unit test (both widths, every boundary) and by
  `tests/regression/sdiv_srem_type_min{,_m32}.c`. Negative powers of two
  (`x % -4`, `x / -8`) were already handled. An earlier note in this
  session listing them as missed was wrong and is withdrawn.

## Verdicts on upstream #979 (`132a75f51`, `f878dca5a`)

**AGREE:** the `target()` ISA plumbing; `sse4.2` implying POPCNT; SHLX mode
selection; the bit-packed switch LUT (a differential fuzz against GCC found
no mismatch); trapv on x86-64; the clone ISA handling; `cpu_probe`'s XCR0
checks; `arch=` dispatch through feature levels rather than GCC's
`__builtin_cpu_is` (more robust on unknown future CPUs, and fail-closed);
keeping the i686 vector-lowering gate closed (see "Open gaps").

**DISAGREE, fixed in this series:**

| # | Defect | Evidence | Commit |
|---|---|---|---|
| D1/D2/D5 | `target("arch=x86-64")` kept AVX2 ymm loops; `no-movbe` still emitted `movbel`; `general-regs-only` FP compiled silently | `-O2 -march=haswell` probes before/after | `e509e2a62` |
| trapv | i686 `mul64` overflow used non-arithmetic I128 | 6776-case matrix per ABI: m32 had 311 wrong results, now 910/910 traps identical to GCC on both ABIs | `7ac5d0f6e` |
| clones | priority table inverted GCC's dispatch order (popcnt<sse4.2, bmi<avx, bmi2<fma, haswell>v3) | gdb runtime table below; old table diverged in 4 of 5 GCC-accepted sets, new matches 5/5 | `6b90f75e5` |
| resolver | `*.resolver` published `next_label=0` below live labels 45/62 | `CCC_VERIFY_IR=abort` on upstream's own fixture failed in every build since #979 | `target_clones: give resolvers a live next_label` |

**DISAGREE, documented, not changed:** D4 (`target("no-sse2")` with `double`
fails closed, where GCC falls back to x87) is the fail-closed policy for an
ABI-changing attribute; D3 (`avx512bw` keeps ymm, where GCC uses zmm) is a
tuning policy, not a defect.

### target_clones runtime differential (gdb: `rbreak ^which`, run, continue, `info symbol $pc`; all clones eligible on the host)

| Clone set | GCC 16.2 | old table | new |
|---|---|---|---|
| fma, bmi2 | bmi2 | fma | bmi2 |
| sse4.2, popcnt | popcnt | sse4_2 | popcnt |
| avx, bmi | bmi | avx | bmi |
| lzcnt, sse4.1 | rejected | sse4_1 | sse4_1 |
| movbe, sse3 | rejected | movbe | sse3 |
| arch=x86-64-v2, arch=nehalem | v2 | v2 | v2 |
| avx2, arch=haswell, arch=x86-64-v3 | v3 | haswell | v3 |

lccc accepts GCC's P_NONE features (lzcnt, movbe, ...) as clones. They are
probe-guarded and rank below every prioritised feature, so a superset of
GCC's accepted inputs dispatches identically on the common subset.

## Verdicts on upstream #972 (`33d948722`)

**AGREE:** `590decd4a`/`d1bd18cc6` (SIMD ties/handoff), `b5a98a230`
(SSSE3), `1fd01c726`/`5f03d6271` (i686 SSE4.1/AVX), `3506a2258` (i686
unwind), `2d3293af4`, `6b8a4b135` (i686 BMI GOT), the IPA-RA algorithm of
`fb4b93320`.
**DISAGREE:** `fb4b93320` ignores `-fno-toplevel-reorder`/`no_reorder`
(GCC keeps top-level order and unreferenced statics). `7addd7f49` and
`85347ab2e` commit binaries over the 100 KB evidence policy; the latter adds
an unreferenced third-party wheel (1,616,141 B) to the repo root with no
license note. Removing it needs a binary patch, so it is left to the owner.

## Defects found in this session's own work and pre-existing code

1. **Call-site vzeroupper** (new feature). GCC 16.2 and Clang 23.1.4 clean
   dirty upper YMM state before ordinary calls; lccc cleaned only at `ret`
   and sibcalls. The first build miscompiled i686: the i686 backend
   overrides `emit_call`, so the trait default that published
   `call_passes_wide_vector` never ran, and the output was
   `vmovups (%esi),%ymm0; vzeroupper; call take`. The gate had passed
   anyway, because i686 PIC functions begin with
   `call __x86.get_pc_thunk.bx`, which satisfied its bare `call` pattern. Both
   are fixed: the i686 override sets the flag (conservatively, then exactly),
   and the gate matches the ABI call by name and was shown to fail on both
   bad builds.
   *Soundness:* no XMM register is callee-saved; the allocator refuses
   call-spanning vector homes; IPA-RA is GPR-only; and deferred vector
   stores are flushed before the call. The flag is never reset after a call,
   because a path that bypasses the call must still return clean.
   *Cost:* +1 instruction per dirty call site, on the call path only. The
   benefit, avoiding the AVX-SSE transition and false dependencies in
   legacy-SSE callees, is a microarchitectural effect that Callgrind cannot
   model and this VM cannot count. The decision rests on the Intel
   optimization manual and on both reference compilers doing the same.
2. **i686 fastcall vector ABI** (pre-existing). GCC passes native vectors
   in `%xmm/%ymm0-2` exactly as cdecl does, and they never touch the ECX/EDX
   chain. lccc put them on the stack, and its callee disagreed with its own
   caller: it captured `%ymm0` but kept the vector's stack bytes, so the
   next int was read from the stack and the function returned with
   `ret $36`. The new gate leg failed on the old compiler in all three
   pairings (GCC to lccc 2, lccc to GCC 2, lccc to lccc 3). The vector rule
   is now a single function, `call_abi::i386_vector_reg_bytes`, shared by
   cdecl and fastcall.
3. **T6 hook inventory drift** (this series). The literal verifier hooks of
   `masked_guard_splat_hoist` and `vec_interleave_late` were never pinned in
   `check_verify_ir_clean.sh`. Each pin is folded into the commit that
   added its pass.

4. **`_mm_stream_ps` was a plain store** (pre-existing). The header body
   `*(__m128 *)p = a` dropped the non-temporal hint. At -m64 it cost a
   register copy and a `push`. At `-m32 -mno-sse` it compiled silently
   into GPR copies, while GCC rejects it. It is now `__builtin_ia32_movntps`
   (new `IntrinsicOp::Movntps`), gated on SSE1 like the other SSE1
   intrinsics. Both backends feed it a `movups` load, because
   `movdqu`/`movupd` are SSE2. `check_legacy_sse_permissions.py` now
   probes the public and raw spellings: refused without SSE; on SSE1-only
   targets, legal and actually `movntps`. Its SSE1 vocabulary now matches
   the driver's (`rcp*`, `rsqrt*`, `movntps`, `cvt*ps*pi`), and
   `rcp`/`rsqrt` are probed too. Runtime: `simd_movnt_sse1{,_m32}.c`.
   Writing that test exposed a worse gate error: `_mm_load{,u}_ps` and
   `_mm_store{,u}_ps` shared the SSE2 `Loaddqu`/`Storedqu` builtins, so
   `-mno-sse2 -msse` refused every SSE1 program that touched memory (GCC
   accepts). They now have their own SSE1-gated identities
   (`X86LoadPs`/`X86StorePs`) on the same IR ops. The i686 backend picks
   `movups` when SSE2 is off, as x86-64's `xmm_move` already did.
   `_mm_sfence` is now SSE-gated too: GCC 16.2 rejects it under `-mno-sse`,
   and on pre-SSE i686 silicon it is #UD. `_mm_clflush` is now SSE2-gated,
   which is GCC's gate. A Pentium III has SSE1 but no CLFLUSH. A census of
   every public SSE..SSE4.2 name that maps to a builtin, checked against
   its header's level, found no other mismatch. The two exceptions are
   `_mm_pause` (ungated, as in GCC) and CRC32 (SSE4.2, as in GCC).
5. **i686 `_mm_stream_si64` of a constant truncated it** (pre-existing).
   The hand-rolled loader stored the low word and a zero high word, so -1
   became 0x00000000ffffffff. It now uses the shared pair loader
   (`emit_load_acc_pair`), which knows every 64-bit operand form.
   `movnti64_i686.c` covers this; it is lccc-only, because GCC has no
   64-bit MOVNTI in 32-bit mode.
6. **`__m64` was not GCC's type** (pre-existing). GCC defines it as an
   8-byte vector, passed in `%xmm0` on x86-64; lccc's definition took the
   integer-class path, so every mixed lccc/GCC call with an `__m64` argument
   or result disagreed. `__m64` is now the GCC vector typedef, the headers
   and the MMX differential generator follow it, and
   `check_public_simd_abi.py` gained `__m64` legs (160 executable legs, zero
   skips; the previous compiler fails the m64 legs, which proves the gate
   catches the fault). i686 follows GCC `-mno-mmx` (memory), because GCC with
   MMX enabled returns `__m64` in `%mm0` without EMMS and corrupts later x87
   code; `-O0 -m32` GCC builds of the fixtures show exactly that divergence.
7. **Small vector call results through wrappers and casts** (pre-existing).
   On x86-64 a call returning a vector of at most 8 bytes yields packed bits;
   `?:`, comma, `({ })` and the same-size vector-to-vector cast forwarded
   them, and every consumer dereferenced them as an address (60
   consumer x wrapper probes segfaulted). The wrappers now spill once
   (`spill_packed_vector_wrapper`), the cast reads its operand through
   `vector_operand_ptr`, and `expr_produces_packed_struct_data` agrees.
   `vector_small_packed_wrappers{,_m32}.c`: GCC `f0f24f4e252b5dd3`, matched
   at -O0, -O2, -O3 -march=x86-64-v3, -m32 -msse2 and -m32 -msse.
8. **i686 `__builtin_shuffle` of at most 8 bytes** (pre-existing). The
   producer packed its result on every target while the consumers, and
   `struct_value_size`, followed the x86-64-only rule; a returned shuffle
   came back as garbage (`f7f55000 f7f55560` against GCC's
   `b00000016 30007`). Producer and every consumer query now share one gate.
9. **`__builtin_shufflevector` handled only 4 x 32-bit** (pre-existing).
   It now has GCC's semantics for any width, including mixed lane counts and
   results narrower or wider than the inputs, with GCC's four diagnostics.
   For Clang `ext_vector_type` the count may be any number (`float4` + 3
   indices is a 16-byte `float3`), as Clang 23.1.4 does; GNU vectors keep
   GCC's power-of-two rule. The 4 x 32 PSHUFD path is retained. Fixtures:
   `shufflevector_any_width` (GCC `a356440faf21d77b`, 6 configurations) and
   the lccc-only, self-checking `shufflevector_ext_vector` (values from Clang
   23.1.4, which also passes it at -O0, -O2 and -m32 -msse2).
10. **`__typeof__` of a shuffle builtin was `int`** (pre-existing): a probe
   summing `sizeof(__typeof__(...))` returned 16; it now returns 40, as GCC
   does.
11. **`_mm_stream_pi` read its vector argument with `lower_expr`** (this
   series, found by red-team before landing): a packed call-result argument
   would have been stored as an address. It uses `vector_operand_ptr`.
12. **Signed-shift UB in the intrinsic headers** (pre-existing). The C
   bodies of the MMX/SSE2/AVX2 set intrinsics built 64-bit lane words as
   `(long long)(unsigned T)x << 32` (also << 48 and << 56), which overflows
   whenever the top lane has its sign bit set: `_mm_set1_pi8(-1)` trapped
   under lccc's UBSan, where GCC's builtin headers run clean. The words are
   now built from unsigned 32-bit halves joined once, which also keeps i686
   out of 64-bit shift chains on register pairs. llvm-mca (100 iterations,
   alderlake / znver4, old -> new): x86-64 `set_pi32` 306/204 -> 109/105,
   `set_pi8` 1507/904 -> 810/605, `set1_pi8` 369/394 -> 112/109,
   `setr_epi16` 721/558 -> 524/561; i686 `set_pi8` 3917/10262 ->
   2119/6141, `set1_pi8` 2718/7048 -> 926/5161, `setr_epi16` 3469/5507 ->
   1785/2273, `set_epi8` 6918/11007 -> 3521/4706. Costs: x86-64 `set_epi8`
   with 16 variable bytes 1563/1613 -> 1624/1715 (+4%/+6%; the constant form
   is identical), i686 `set_pi32` 1118 -> 1218 on alderlake (znver4 equal).
   Rejected alternatives, with data: a vector literal (`(__v8qi){...}`)
   stores bytes and reloads a qword, a store-forwarding failure on both
   cores; an unsigned 64-bit chain is UB-free but slower than the halves on
   every target. `simd_set_lanes_ubsan{,_m32}.c` run every set form with
   negative, positive and minimum top lanes under UBSan trap mode against
   GCC; the old headers trap (SIGILL) at -O0, -O2 and -m32.
13. **SSE1-only XMM transfers of 8-byte SSE-class values** (this series:
   making `__m64` a vector exposed it; also pre-existing for `float x2`
   and SSE-class struct eightbytes). The parameter capture store, the
   single- and two-eightbyte call-result stores used MOVQ/MOVSD
   (SSE2) between XMM and memory, so `-mno-sse2 -msse` rejected
   `void f(__m64 *p, __m64 a) { _mm_stream_pi(p, a); }`, a case
   `check_legacy_sse_permissions.py` requires to compile. They use MOVLPS
   (SSE1, the same eight bytes) without SSE2, as GCC does.
   `vec8_abi_sse1_only{,_o0}.c` pass `__m64`, `float x2`, `int x2` and
   `{__m64}`, `{F2, long}`, `{long, F2}`, `{F2, F2}` through calls, as
   results, through a function pointer and as a stack argument
   (GCC `6265d4f34ddddfda`). The fix lands before the `__m64` switch with
   the non-`__m64` shapes (bisectable: built at that commit, they match
   GCC and the permissions gate passes); the `__m64` shapes land with it.

Items 6-13 are correctness fixes. Validation of s171 on `5cca27779`: corpus
1228/0 with every verifier, ABI gate 160/160, MMX differential 72/72 on
seven flag sets, shuffle diagnostics, driver contract, inventories and
generators (`--check`), rustfmt and clippy strict.

Items 4-5 and the type-minimum fold leave all 56 registered benchmark
workloads assembly-identical at -O2, -O3 -march=x86-64-v3 and -O2 -m32
(`cgsweep.py`, s162 -> s164). They are correctness and permission fixes
with no hidden cost. Validation of s164: lib 6521/0, corpus 1208/0 (all
verifiers), ring identities, fuzz_identities 100/0, the 11 surgical gates,
fuzz 960/0 in both modes, clippy strict, rustfmt, and CI gate parity.

## Measurements (Callgrind Ir, millions)

| Workload | lccc | GCC 16.2 | Clang 23.1.4 | ICX 2026.2 |
|---|---|---|---|---|
| `bench.c` AVX2 | 42.27 | 109.88 | 42.14 | 52.50 |
| `bench.c` SSE2 | 106.99 | 352.36 | 136.49 | 133.32 |
| zstd `count` | 93.87 | 86.40 | 95.05 | |
| zstd `double_fast` | 2823.5 | 2686.2 | 2460.1 | |

## Open gaps (measured, not yet closed)

* **i686 GNU vector arithmetic is scalarised.** `driver/pipeline.rs` sets
  the vector-lowering width to 0 for non-x86-64, and the i686 backend
  implements about 12 of roughly 202 `Vec*` families. `-O2 -mavx2`
  add-instruction counts, m32 vs m64: char32B 173/5, short32B 93/5, int32B
  53/5, ll32B 65/5, int16B 33/4, ll16B 37/4, float32B 81/5, double32B 45/5.
  GCC m32 needs 3 instructions. Opening the gate without the families would
  miscompile, so the gate stays closed. The fix is porting the families,
  ideally sharing the x86-64 emitters.
* **Ir gaps to GCC 16.2:** reduction_vecreg 3278 vs 1721, popcount 5801 vs
  4728, nbody 3385 vs 2965, switch_dispatch 978 vs 766, zstd `double_fast`
  2823.5 vs 2686.2.
* **Unsigned-exit reductions** (`for (size_t i...) s += x[i]`) stay scalar
  under upstream `cdae9c617`'s Slt/Sgt restriction. A census over 74
  benchmark programs at {-O2, -O3 -march=x86-64-v3} found 0 hits, so the
  restriction stays (AGREE) and the gap is recorded as out-of-corpus.
