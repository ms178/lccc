# Follow-up: assembler encoding quality

Session work (2026-09-09) against `ms178/lccc` `91ecc08`. Goal: shortest
*correct* encoding. GAS 2.47 is secondary. Primary oracles are the
integrated assemblers of Clang 23.1 (`cclang2310`), latest ICX
(`cicxlatest`), ICC 2021.10 (`cicc2021100`), GCC 16.2 (`cg162`), via
`scripts/encdiff.py` / `scripts/godbolt.py`. Performance is primary,
size is secondary — a shorter legal encoding is an I-cache/decode win,
and folding indexed addressing is also a µop win (below).

## Landed this session

- **VEX2 source-swap** on remaining 0F-map integer AVX: `vpmuludq`,
  `vpsadbw`, `vpmaddwd`, plus VEX `vpmulhuw` (was EVEX-only, so xmm/ymm
  `REJECTS-VALID`). Same change on i686. `avx_int` cases use high/high or
  low/low operands so they stay byte-identical to GAS; the swap win lives
  in the hand-tagged `vex2_commutative betterok` group. Also commutative
  on x86: `vpaddusb`/`vpaddusw`/`vpaddsb`/`vpaddsw`, `vpcmpeqw`,
  `vpminub`/`vpmaxub`/`vpminsw`/`vpmaxsw`.
- **Missing VEX arms** (xmm/ymm): `vpmulhrsw`, `vphsubw`, `vphsubd`,
  `vphaddsw`, `vphsubsw`, `vpmuldq`, `vmpsadbw`, `vphminposuw`. EVEX zmm
  path unchanged. Covered by new `avx_ssse3` asmdiff group (must match GAS).
- **i686 scale-1 SIB fold**, duplicated locally in `i686/assembler/encoder/core.rs`
  (x86 `fold_scale1_index` is `pub(crate)` in a private module). Skips
  `%esp`/`%sp`; runs before the `.code16` ModR/M table.
- **Unsuffixed `cvtsi2ss`/`cvtsi2sd`** now call `encode_sse_cvt_gp_to_xmm`
  with `infer_reg_size` (mem default 4). Fixes double-REX, `%r8d` falsely
  64-bit, and missing REX.R for `%xmm8`.
- **`extractps`**: `emit_rex_rr`/`emit_rex_rm` so `%xmm8`/`%r8d` get REX.R/B;
  RIP addend adjusted for the trailing imm8.
- **Scripts**: `encdiff.py` Clang oracle `cclang2210` → `cclang2310`;
  `--json` scoreboard + head-to-head byte table vs every oracle;
  `_COMMUTATIVE_VEX` extended; `gen_encoding_sweep.py` families
  `vex_commutative`, `avx_ssse3`, `sse_rex` **wired into `GROUPS`**
  (the functions existed but `--list` / default sweep omitted them);
  `gen_asmdiff_corpus.py` updated but **avx/lea/modrm/misc casefiles
  were not regenerated** (would drop hand-tagged `betterok`).

## Oracle audit (`godbolt.py audit`, 2026-09-09)

| alias | pinned id     | pinned    | newest    | status |
|-------|---------------|-----------|-----------|--------|
| gcc   | cg162         | 16.2      | 16.2      | ok     |
| clang | cclang2310    | 23.1.0    | 23.1.0    | ok     |
| icc   | cicc2021100   | 2021.10.0 | 2021.10.0 | ok     |
| icx   | cicxlatest    | (latest)  | moving    | ok     |

## encdiff vs every integrated assembler (data)

Two sweeps, LCCC `target/fastbuild/lccc`, oracles Clang 23.1 / ICX latest /
ICC 2021.10 / GCC 16.2 / GAS 2.47. **Zero `LONGER`, zero `WRONG-BYTES`,
zero `REJECTS-VALID`.** Exit 0 on both.

Focus sweep (`sib_fold vex_len vex_commutative lea_strength sse_rex
avx_ssse3 imm8_sext shift_one xchg_acc disp_shrink`): **1717** insns,
`ok-best=306  ok=1402  DECLINED-FP=8  DECLINED-WRONG=1`.

| oracle | beat | tie | lose | n    | LCCC B | them B | delta |
|--------|-----:|----:|-----:|-----:|-------:|-------:|------:|
| clang  |  221 |1488 |    8 | 1717 |   7932 |   8668 |  −736 |
| icx    |  221 |1488 |    8 | 1717 |   7932 |   8668 |  −736 |
| icc    |   79 |1637 |    1 | 1717 |   7932 |   8010 |   −78 |
| gcc    |  287 |1430 |    0 | 1717 |   7932 |   8742 |  −810 |
| gas    |  287 |1430 |    0 | 1717 |   7932 |   8742 |  −810 |

The 8 clang/icx "loses" are `DECLINED-FP` (`vaddps`/`vmulps`/`pd` VEX2
source-swap). The 1 ICC "lose" is `DECLINED-WRONG` (`xchg %eax,%eax` →
`0x90`). Neither is a legal shorter equivalent.

Rest of `GROUPS` (`accumulator test_and mov_imm …`): **1017** insns,
`BEATS=30  ok-best=20  ok=967`. Lose = 0 vs every oracle.

| oracle | beat | tie | lose | n    | LCCC B | them B | delta |
|--------|-----:|----:|-----:|-----:|-------:|-------:|------:|
| clang  |   30 | 987 |    0 | 1017 |   4594 |   4728 |  −134 |
| icx    |   30 | 987 |    0 | 1017 |   4594 |   4728 |  −134 |
| icc    |   33 | 984 |    0 | 1017 |   4594 |   4641 |   −47 |
| gcc    |   30 | 987 |    0 | 1017 |   4594 |   4728 |  −134 |
| gas    |   30 | 987 |    0 | 1017 |   4594 |   4728 |  −134 |

**Combined (2734 insns):** LCCC 12526 B vs clang/icx 13396 (−870), vs
ICC 12651 (−125), vs GAS/GCC 13470 (−944). LCCC is the shortest *correct*
assembler of the five on this corpus.

Who we beat and how:

- **vs Clang 23.1 / ICX:** 221 SIB-fold `mov`/`lea` (`(,%reg,1)` →
  `(%reg)`). They match us on VEX2 integer commutative. They "win" only
  on the 8 declined FP swaps.
- **vs ICC:** 79 VEX2 commutative + AVX mov-shortform (ICC still emits
  VEX3). They match us on the SIB fold. They "win" only on `xchg %eax,%eax`.
- **vs GAS 2.47 / GCC 16.2:** both families. Secondary.
- **`BEATS=30`** on `movq $0x80000000/%0xffffffff, %r64`: LCCC emits
  zero-extending `mov r32, imm32` (5–6 B). Clang/GAS/ICX emit 10 B
  `movabs`. ICC emits 7 B `REX.W mov r64, imm32` which **sign-extends**
  `0x80000000` to `0xffffffff80000000` — wrong for the unsigned value.
  Existing encoder, not a new shrink; do not extend this to kernel-sized
  `movabs*`.

Compiler-output check: `lccc -O2 -S tests/benchmark/programs/gzip_crc32.c`
→ 75 unique insns, encdiff `ok=75`, byte-identical to every oracle that
assembled them (clang/icx skip LCCC local labels, as expected).

## Performance (uops.info), not just size

Scale-1 index fold is not only −1 byte. uops.info `ADD_M64_R64`:

| uarch  | retire slots (base) | indexed | loop tp (cyc/insn)     |
|--------|--------------------:|--------:|------------------------|
| SKL/SKX/CLX | 2              | 3       | 1.00 / 1.00            |
| ICL    | 2                   | 3       | 0.53 / **0.67 indexed** |
| ADL-P  | 2                   | 3       | 0.54 / 0.53            |
| MTL-P  | 2                   | 3       | 0.75 / 0.78 indexed    |
| ARL-P  | 2                   | 3       | 0.57 / 0.58; MITE 2 vs **3** |

Indexed addressing is +1 retire µop on every Intel P-core in that table,
and slower in the ICL loop. Folding `(,%rax,1)` to `(%rax)` is the
performance win; the byte is the I-cache bonus.

VEX2 vs VEX3 is the same execution µops (e.g. `LEA_B_R64` vs
`LEA_B_I_R64` on SKL are both tp 0.50) — size/decode only, which is why
size is secondary to the SIB fold.

`hot_loop_metric.py` on LCCC `-O2` output (compile-test, not an
assembler differential): `gzip_crc32_update` 8 insn / 1 B/trip;
`stencil5` 11 insn / 32 B/trip, 256-bit.

## Compile-testing

- `asmdiff.py` vs GAS 2.47: **788 passed, 0 failed** (x86-64, includes
  `tests/asm-diff/evex.casefile`). i686: **5 passed, 0 failed**
  (`asmdiff.py --32 --lccc target/fastbuild/lccc-i686`).
- `encdiff.py` remote: 2734 insns, exit 0 (see tables).
- `lccc -O2 -c tests/benchmark/programs/gzip_crc32.c` → 0.
- `.github/scripts/ci-codegen-gate.py`: all 7 golden workloads PASS
  (gzip 77, adler 234, expat 203, varint 302, memcmp 206, hash 147,
  stencil5 113 insns).

## Do not

- Do not swap sources of `vaddps`/`vmulps` (and pd/scalar) for a VEX2 byte:
  `DECLINED-FP` (NaN payload is SRC1).
- Do not treat `xchg %eax,%eax` as `0x90`: `DECLINED-WRONG`.
- Do not shrink `movabs*` of kernel-sized immediates.
- Do not implement 0F38 commutative VEX2 swap: 2-byte VEX requires mm=1.
- Do not `use` x86 `encoder::core::fold_scale1_index` from i686.
- Do not regenerate `tests/asm-diff/{avx,lea,modrm,misc}.casefile`.

## v3 (this session)

Correctness bugs, not quality nits. GAS 2.47 byte-identical on the new
corpus; Clang 23.1 / ICX / ICC / GCC 16.2 tie on `evex_quality` (108 B
each, 17 insns). Policy: **never prefer EVEX when VEX can encode**
(xmm/ymm0–15 stay VEX2/VEX3). EVEX only for zmm, xmm/ymm16–31, k/mask/z,
broadcast/SAE, or `evex_only` mnemonics.

- **High vector registers 16–31.** `vec_reg_id` is 0–31; `evex_id`
  packs R/R′/X/V′. `operand_needs_evex` gates EVEX. If the mnemonic has
  no EVEX form (`vpxor`/`vmovdqa` xmm16) we **reject**, matching GAS
  (`no EVEX encoding for vpxor`). We do **not** fake EVEX for those.
- **ModRM.reg=0 dest bug** on `encode_evex_imm2` / `imm2_ndd` /
  `3src_imm` / `broadcast_gpr`: dest was always zmm0. Now dest is
  ModRM.reg (`vpshufd $1,%zmm2,%zmm3`, `vpternlogd`, `vpbroadcastd %eax,%zmm3`
  match GAS).
- **Compressed disp8×N** by tuple, not VL: Full = 16/32/64; Tuple1
  vpbroadcastd/q N=4/8; Tuple4 `vbroadcasti32x4` N=16; Tuple8
  `vbroadcasti32x8` N=32. `vpxord 64(%rdi), %xmm0` uses N=16.
  `vpmovzx*` still Full (safe-long disp32 vs GAS Half).
- **`vphminposuw` ymm** rejected on x86 and i686 (GAS: operand size
  mismatch). xmm form unchanged.
- **EVEX xmm/ymm coverage** for FP bitwise/arith/mov that GAS encodes
  as EVEX for xmm16–31: `vandps`/`vandpd`/`vorps`/`vxorps` + n/pd,
  `vaddss`/`sd` and family, `vmovaps`/`apd`/`ups`/`upd`. xmm0–15 still
  VEX (`vaddps %xmm0,%xmm1,%xmm2` = `c5 f0 58 d0`).
- **i686 asmdiff**: ELF32 reader, `as --32`, `--32` flag, corpus
  `tests/asm-diff/i686/basic.casefile` (ALU/SSE/AVX/scale-1 betterok /
  vphminposuw ymm reject).
- **Scripts**: `gen_encoding_sweep.py` group `evex_quality`; asmdiff
  ELF32. Do not add asmdiff to `ci_local.sh` without `LCCC_GAS`.

Validation: asmdiff 788/0 (x86-64) + 5/0 (i686). encdiff `evex_quality`
17/17 tie vs clang/icx/icc/gcc/gas. Focus+evex_quality offline vs GAS:
1733 insns, lose=0, BEATS=287, −810 B.

## Next (still open)

1. **pmovzx/sx tuple type** still Full (disp32) vs GAS Half (disp8×N).
   Safe (same address) but longer. Per-mnemonic N table.
2. **Remaining EVEX mem/mask paths** (cmp-mask mem, rotate mem, k-mask
  on more mnemonics, SAE/embedded rounding). High-reg R′ on leftover
  VEX-era helpers that still use `needs_vex_ext` (GPR-only, fine).
3. **BMI / APX** encoding quality: `andn`/`sarx` cannot VEX2 (0F38).
   APX NDD/EGPR not started.
4. **`ci_local.sh` full** (not `--fast`) on the final patch. Fastbuild
   only (`-O1 -j2`). Rebase onto latest `ms178/lccc` main before the
   last snapshot. Do not add asmdiff to CI without `LCCC_GAS`.

## Validation recipe

```bash
export LCCC_GAS=/home/user/.cache/gas-2.47-x86_64/bin/as
export LCCC_OBJCOPY=/home/user/.cache/gas-2.47-x86_64/bin/objcopy
export LCCC_OBJDUMP=/home/user/.cache/gas-2.47-x86_64/bin/objdump
export LCCC_BIN=/home/user/lccc/target/fastbuild/lccc

scripts/build_lccc_fast.sh
python3 scripts/godbolt.py audit
scripts/asmdiff.py --as "$LCCC_GAS" --lccc "$LCCC_BIN"
python3 scripts/gen_encoding_sweep.py -o /tmp/sweep.s
python3 scripts/encdiff.py --file /tmp/sweep.s --lccc "$LCCC_BIN" \
    --json /tmp/encdiff.json
python3 scripts/uops_info_probe.py --arch SKL,ICL,ADL-P,ARL-P \
    ADD_M64_R64 LEA_B_R64 LEA_B_I_R64
python3 .github/scripts/ci-codegen-gate.py --lccc "$LCCC_BIN" --summary
scripts/ci_local.sh --fast
scripts/lccc-snapshot.sh oracle-encdiff-scoreboard \
    "encdiff JSON scoreboard vs Clang 23.1/ICX/ICC; GROUPS wired"
```
