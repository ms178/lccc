# VEX/EVEX/XOP encoding perfection campaign (2026-09-25, session B)

Oracle: GAS 2.47 (self-built x86_64 + i686) byte-for-byte; `isa_gap_sweep.py`
--full rank tables for prioritization; `asmdiff.py`/`encdiff.py`/`insndiff.py`
for whole-object, oracle-fleet, and single-insn differentials. Every opcode,
slot placement, decorator rule, and reject message below was probed against
the oracle before implementation — none taken from the SDM from memory (the
SDM memory was repeatedly wrong: vcvtph2ps pp is 66 not F3; vcvtps2ph VEX pp
is 66; vshufi32x4's map is 0F3A for the 23-opcode pair, vdbpsadbw too).

## Scoreboard

| metric | session start | final |
|---|---|---|
| x86-64 testsuite-line PASS (122508 lines) | 37614 | **41206** (+3592) |
| i686 PASS | 17969 | **20237** (+2268) |
| FALSEACC (LCCC accepts, GAS rejects) | 0 | **0** (both archs) |
| asmdiff cases | 867 | **900** (33 new regression cases) |
| ci_local.sh --fast | — | **68 passed / 0 failed / ALL GREEN** |
| clippy -D warnings / rustfmt --check | — | clean |
| unit tests | 3431 | **3435 passed / 0 failed** |

## What landed (snapshots S05–S12)

1. **S05/S06**: in-flight work preserved + re-based onto origin/main
   `68484ebb` (PR #618, disjoint file sets; deliverable byte-identical).
2. **S07 xop-memsrc2**: the 4-op XOP memory-src2 form (vpcmov/vpperm/vpmac*)
   — reg = dst, r/m = src2 (memory), vvvv = src3, imm8[7:4] = src1 — all
   three families accept it (byte-verified); vpermil duplicate-arm fix.
3. **S08 xop-full-vpermil2**: vfrcz{ps,pd,ss,sd} (the ymm-capable XOP unary,
   mem-capable), vpsha*/vpshl* (xmm-only map-9 binaries), vpermil2ps/pd
   (five-operand VEX.66.0F3A 48/49: imm8 = (slotless_reg << 4) | selector,
   W=1 ⇔ memory-first-source, `constant doesn't fit in 4 bits` parity), and
   the diagnostic taxonomy: ymm-capable families reject mixed widths with
   `register type mismatch`, xmm-only families (vpperm/vprot*/vpmac*/vph*)
   reject any ymm with `operand size mismatch`; LWP rejects vector regs.
4. **S09 evex-cluster1**: vrcp14ps/pd + vrsqrt14ps/pd (unary 4C/4E, W
   selects), vexpandp*/vcompressp* (register stays in ModRM.reg for BOTH
   directions; half-mem tuples), vptestm* + vptestnm* (k-DESTINATION
   family: vvvv = 2nd-listed, r/m = 1st-listed/mem — GAS's placement is
   the reverse of the scalar-control family — {k5} input mask on the k-dest
   allowed, {z} rejected), 18 vpmov* down-conversions (ModRM.reg = wide
   SOURCE, LL from source, N = lanes×elem, dst-width class rule: zmm→xmm
   for qd is `operand size mismatch`), vshuf{f,i}32x4/64x2 (no xmm forms in
   AVX512F; imm8 full byte range; broadcast forbidden), vdbpsadbw,
   vcvtps2ph (VEX pp=66! reg=src for BOTH mem and reg destinations;
   EVEX LL from source, mem N=lanes×2), vcvtph2ps (VEX pp=66; EVEX map 2,
   Wide, N=2).
5. **S10 evex-cluster2**: variable vpermq/vpermpd (EVEX.0F38.W1 36/16 — no
   memory spelling, `operand type mismatch`; the $imm form stays VEX — dual
   dispatch in the VEX table), vmovddup/vmovshdup/vmovsldup EVEX forms,
   vbroadcasti32x2 (reg-src unary + fixed N=8 mem tuple), vpblendmb/w,
   vpsllvw/vpsrlvw/vpsravw (opcodes 12/10/11 — the distill battery caught a
   10/11 swap), broadcast-forbidden on the w-granularity shifts.
6. **S11 case-and-segments**: Register::new lowercases (GAS is
   case-insensitive: `%FS`, `%RAX`, `VMOVDQA` — every encoder table keys on
   lowercase), i686 delegation case fix (1138 uppercase `V*` lines were
   MISSING on i686 — the gate matched case-sensitively), masked
   vmovss/vmovsd EVEX (merge/load/store with full {k}{z}; the two-register
   spelling is `operand type mismatch` — a pre-existing FALSEACC fixed),
   and the push/pop segment fix: `push %fs` was silently encoding as
   `push %rsp` (0x54) — a wrong-code bug, now `0f a0` with es/cs/ss/ds
   rejected verbatim (`you can't `push %es'`).
7. **S12 regression-casefiles**: tests/asm-diff/xop-vpermil2.casefile +
   evex-avx512f-dq-bw.casefile (33 cases) lock the byte-exactness into CI;
   the auto-globbing CI run is 900/900 (x86-64) + 26/26 (--32).

## Oracle fleet (encdiff, 1373-instruction corpus, payload bytes)

| oracle | beat | tie | lose | note |
|---|---|---|---|---|
| GAS 2.47 | 118 | 1249 | 0 | shorter encodings, never worse |
| GCC | 118 | 1249 | 0 | |
| ICC | 130 | 1237 | 0 | |
| Clang | 4 | 1343 | 20 | all 20 = DECLINED-FP: the shorter form
swaps FP sources and would change NaN payload propagation — refused by
design, not a defect |
| ICX | 4 | 1285 | 20 | same DECLINED-FP set |

insndiff probe corpus: 69 ok, 1 BETTER (legitimate sign-extension
shortening), 0 divergences.

## Engineering notes for the next session

* The i686 vector delegation (`delegate_vector_to_x64`) is now
  case-insensitive and covers every VEX/EVEX/XOP/LWP family; GP-dest
  families (vcvtsi2ss et al.) stay local by design, and the *usi family
  (23+23 i686 lines) is the remaining delegation candidate — the x64 core
  encodes GP ids < 8 byte-identically, so a `delegate-despite-GP` list for
  EVEX-only GP-dest mnemonics is the correct next move.
* `mov` (278 x86-64 / 278 i686) is the top-ranked legacy gap: 209 lines are
  segment-override memory operands with redundant-scale SIBs
  (`mov %ds,%ds:(%eax,1)`), 17 are `sym@size` relocations, plus
  `addr32/addr16` (72/39) force-67/66-prefix branches. TBM
  (blcfill/blcic/blcmsk/blcs/blsfill/blsic/t1mskc/tzmsk/blci, ~450 lines)
  is a compact legacy win.
* Sweep temp hygiene: `isa_gap_sweep.py` leaves `/tmp/isa-sweep-*` behind on
  crash; prune after runs (disk-full killed two sweeps this session).
* OOM discipline on this 4.1G/no-swap sandbox: `cargo clippy` needs
  CI_LOCAL_JOBS=1 (gate SIGKILLed at -j2), `cargo test` needs the script's
  dbg=0/incremental=0/jobs=1 path (default on <6G hosts). Detached CI runs
  die silently mid-compile — run heavy gates in foreground tool calls.
