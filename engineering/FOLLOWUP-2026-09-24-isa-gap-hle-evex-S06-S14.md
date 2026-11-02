# ISA gap campaign S06–S14: `.s` parity, HLE prefixes, EVEX converts/shifts (2026-09-24)

Oracle: GAS 2.47 (self-built, `--64`/`--32`) + `objdump` disassembly bytes.
Deliverable: `/home/user/ms178-1.patch` (109987 bytes, applies-clean on
`95e46ab`), snapshots S06–S14 via `lccc-snapshot.sh`.

## Scoreboard (full sweep: every testsuite line individually, `--full`)

| point | pass | fail-lines | fail-mnemonics |
|---|---|---|---|
| S05 baseline (3-line sample mode) | 3233 | 2672 | 1009 |
| S10 full-mode ground truth | 35372 | 29388 | 1128 |
| **S14 full-mode** | **37278** | **27482** | **1127** |

S10→S14 delta: **+1906 passes / −1906 fail-lines** (HLE ~958, vpermil 340,
vpconflict 188, EVEX shift-count ~580; remainder is testsuite-line overlap).
Corpus: asmdiff 847 passed / 3 failed — the 3 fails are the pre-existing set
(`apx_gotpcrel_code6`, `apx_gotpcrel_not_code6`, `conditionals`), unchanged.
i686 gate: 21/21. `ci_local.sh --fast`: not re-run this session (run before
deliverable).

## S06 dot-s-suffix — GAS `.s` parity + short branches

- `.s` length suffix: strip-and-accept for instructions (reg-reg D-bit
  direction difference vs GAS documented and deliberately not threaded
  through), prefix-exclusion for standalone `lock.s`/`rep.s`/… (GAS rejects).
  x86-64 + i686 (`elf_writer`, `elf_writer_common`, i686 `encode_mnemonic`).
- Jump relaxation had **two** gates, not one: the arch jump detector AND
  `get_jump_target_label`. Uppercase `JMP` bisected via `Jmp`→E9 vs `jMp`→EB
  to the second gate; both normalized (lowercase minus `.s` rule).
- New: `jecxz` (`67 E3`), `loope/loopz` (E1), `loopne/loopnz` (E0); `jcxz`
  correctly rejected in 64-bit. misc 10/10, 9-line short-branch battery
  byte-identical incl. addresses.

## S07 evex-fma-addsub — EVEX FMA addsub/subadd

- 12 arms: `vfmaddsub/vfmsubadd{132,213,231}{ps,pd}` zmm
  (map2/pp1, W0 ps / W1 pd; opcodes 96/97/A6/A7/B6/B7).
- VEX ymm forms already worked through the `fma3_opcode` table — only EVEX
  dispatch was missing. 24-line battery byte-identical; evex+avx 36/36.

## S08 vex-pmovzx-qword — VEX pmovzx/sx quadword dests

- Only `*bq`/`*wq` were truly missing; a truncated `grep | head -25` hid the
  other 8 pre-existing arms and caused a duplicate-arm compile failure.
  Fixed by **unifying** into one 12-arm block, not appending.
- 36-line reg+mem+xmm/ymm battery byte-identical.

## S09 vex-gfni — VEX GFNI (the W1 surprise)

- `vgf2p8mulb` (W0) + `vgf2p8affineqb/invqb`: the affine forms are **W1**
  (`c4 e3 d1 ce …`), not W0 — caught by the oracle battery, generalized
  `encode_avx_3op_3a_pp_imm8` with a `w` param (vpclmulqdq passes 0).
- 9-line battery byte-identical. (Second `w`-generalization lesson of the
  session: never trust memory of W bits; always battery first.)

## S10 evex-cvt-usi — EVEX unsigned scalar converts (the EVEX-only surprise)

- `vcvt{,t}{sd,ss}2usi` + `vcvtusi2{sd,ss}{,l,q}` are **AVX512F-only**: GAS
  emits EVEX unadorned (`62 f1 7f 08 79 c0`). No VEX forms exist.
- New helpers `encode_evex_cvt_to_gp` (0x78/0x79, W from dst width) and
  `encode_evex_cvt_from_gp` (0x7B, W from suffix with r64/mem inference);
  LIG, vvvv=1, scalar disp8 tuples (sd=8/ss=4, m32=4/m64=8).
- No masking/broadcast/SAE on any of them (all oracle-rejected) — dispatch
  chains `evex_forbid_mask_bcst`, helpers peel `{sae}` for the exact GAS
  message. All 10 mnemonics added to the `evex_only` routing allowlist
  (without it `try_encode_evex` is never consulted for xmm/GP operands).
- 30-line battery byte-identical + reject parity.

## S11 hle-prefix-stacking — HLE + stacked prefixes (958 fails, top win)

- `Instruction.prefix: Option<String>` → `prefixes: Vec<String>` (parser,
  x86 encoder, i686 encoder, numeric_labels, 2 i686 test literals).
- Canonical order F2/F3-before-F0 regardless of source order; standalone
  segment overrides + `notrack` ride **pre-body** (fixes the pre-existing
  `gs addw` misorder: was `66 65`, now `65 66` like GAS); group-1 run
  spliced after seg/66/67 with multi-byte reloc shift.
- Validation with verbatim GAS diagnostics: `expecting lockable instruction
  after 'lock'` (RMW/bittest/xchg/xadd/cmpxchg stems + memory operand),
  `missing 'lock' with 'xacquire'/'xrelease'`, `instruction 'mov' after
  'xacquire' not allowed`, `memory destination needed … after 'xrelease'`,
  `invalid instruction '<stem>' after 'x…'`, `same type of prefix used
  twice` (byte-equality: covers `rep repe`).
- 22-line accept battery byte-identical (incl. `gs rep movsb`, `gs addw`),
  12/12 reject messages identical, existing `rep`/`notrack` laxity
  preserved, 10 new reject cases in prefix.casefile.
- i686 HLE (`lock xacquire adcb`, …) works as a free bonus via the shared
  parser + a 10-line prefix loop; 32-bit battery byte-identical.

## S12 evex-vpermil — EVEX vpermilpd/ps (340 fails)

- Two `encode_evex_imm2` arms (map3/pp1, W1/05 + W0/04); routing automatic
  via zmm/masked operands. Mask/broadcast/diverse-LL battery byte-identical.

## S13 evex-vpconflict — AVX512CD (188 fails)

- Two `encode_evex_unary` arms (map2/pp1, W0/W1 0xC4). Battery byte-identical.

## S14 evex-shift-count — EVEX shifts by xmm/m128 (580 fails, 9 mnemonics)

- `encode_evex_shift` dispatcher (mirrors VEX `encode_avx_shift`): `$imm`
  → imm-group path, else count-in-r/m (`ModRM.reg` = dst, vvvv = src).
- Oracle corrections: q-forms use distinct opcodes (`vpsllq`=F3,
  `vpsrlq`=D3; only `vpsraq`=E2 shares with `vpsrad`); count tuple is a
  fixed N=16 at every VL; broadcast → `unsupported broadcast for '<m>'`
  (new `evex_forbid_broadcast` sibling); ymm/zmm/GP count → `operand type
  mismatch for '<m>'` (helpers take `mnemonic`, gp_integer precedent).
- 22-line battery byte-identical + 3/3 rejects; imm-form arms preserved
  through the dispatcher (evex.casefile 20/20).

## Sweep infrastructure

- `isa-gap-sweep.py`: added a ranked `FAILS RANKED` table and `--full` mode
  (tests every line individually for true fail counts; ~160 s at `--jobs 2`).
  Sample mode caps at 3 lines/mnemonic and cannot rank — always use `--full`
  for prioritization.

## Next targets (ranked, S14 full sweep)

1. `vpcmov` 129 — XOP (needs XOP prefix machinery; biggest single item).
2. `lwpins`/`lwpval` 121+121 — AMD LWP (legacy 8F-path, likely easy).
3. `vpmadd52huq/luq` 100+94 — AVX512_IFMA VEX+EVEX (easy arms).
4. `vcvtpd2qq`-family 96–89s — missing EVEX unary arms (easy, vpconflict-like).
5. `vprotb/w/d/q` 96 — VEX forms unhandled + EVEX (medium).
6. `vfixupimm/vgetmant/vrange/vreduce/vrndscale/vscalef` 96–90 — diagnose
   per-mnemonic (likely EVEX decorator gaps on existing arms).
7. `vpslldq/vpsrldq` 2+2 — inspect (imm-helper gap at some VL?).

## Deferred / follow-up notes

- `rep`/`notrack` validation (`invalid instruction after 'rep'`,
  `expecting indirect branch instruction after 'notrack'`) deliberately not
  added: no sweep coverage, existing laxity preserved; `notrack callw/jmpw`
  needs the missing `callw/jmpw` (16-bit) instructions first.
- i686 instruction backports (FMA-addsub, pmovzx-q, GFNI, USI converts,
  vpermil, vpconflict, shift-count): x86-64 only this session; i686 HLE
  came free, the rest is open.
- EVEX `{sae}`/`{er}` on from-GP converts: GAS rejects (`misplaced`), no
  work needed. `vmovw` FP16 gap still open.
- `xacquire`+`xrelease` together, `rep`+`lock` stacking: uncovered corners,
  byte-rule accepts; GAS behavior unprobed.
