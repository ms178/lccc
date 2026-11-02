# Session follow-up: VEX/EVEX red-team audit + EGPR/GPR-r/m encoding fixes (ms178-1)

**Base:** `e5bc19118c6ea5a54d5539d4a6cff52c97ab6baa` (ms178/lccc main, includes PR #627)
**Deliverable:** `/home/z/my-project/ms178-1.patch` (+ `/home/z/my-project/artifacts/`)

## Audit verdict on the merged VEX/EVEX PRs

Audited: c1c6736 (#623), 3b0c672, cde38d5 (#624), 413bdf7 (#627).
Method: `encdiff.py` round-trip verified over the ENCDIFF subset of the
binutils-2.47 testsuite sweep (658 lines, oracles: GAS 2.47 + clang + gcc +
icc + icx via godbolt), `isa_gap_sweep.py` x86-64+i686 (122,508 lines each,
FALSEACC=0), targeted GAS byte-probes, binutils source cross-reading
(`build_apx_evex_prefix`, `OP_EX`, `print_register`).

**AGREE** with the core designs: {evex} dispatch-before-soundness-gate,
promoted-row tables (byte-probed, spot-verified), decorator duplicate
detection, vpextrw C5 field order, vmpsadbw AVX10.2 row, sized symbol
immediates, scalar {1toN} rejection classes. **DISAGREE** on four points,
all fixed in this session:

1. **[miscode, fixed] GPR r/m ≥ r16 in EVEX mod=3 used the VECTOR extension
   bit.** `vcvtsi2sdq %r21,...` encoded `%rbp`; `vextractps $1,%xmm16,%r20d`
   encoded `%esp` (oracle-disassembler-confirmed). Hardware law: vector r/m
   +16 ← EVEX.X (P0 bit6, inverted); GPR (EGPR) r/m +16 ← rex2.B (P0 bit3,
   SET, non-inverted). The memory path (`evex_addr_bits`) already had the
   GPR law — `emit_evex_mod3` was forgotten. Fixed class-aware
   (`is_gpr_name`), reusing the established `apply_evex_apx_addr` mechanism.
   Root cause of PR #624/#627's blind spot: their batteries only used
   r8-r15 GPRs (B3 coverage, no B4 coverage).
2. **[miscode, fixed] vmovddup memory tuple was full-VL; xmm form is Tuple1
   m64 (N=8), ymm/zmm are Full (N=32/64).** `vmovddup -1024(%rdx),%xmm30`
   computed disp8 -64×16 = -512 (WRONG ADDRESS; GAS: -128×8). PR #627's own
   `evex_scalar_tuple_n` table classified vmovddup as N=8 — but only the
   decorator rejection consumed it, not the displacement encoder: the table
   and the encoder disagreed inside one PR. New
   `encode_evex_unary_tuple1` override mirrors the binary family's
   `scalar_n` pattern; dispatch selects 8/32/64 by destination class.
3. **[quality, fixed] VEX move direction never tried the mirrored opcode.**
   `vmovss/sd` 3-op and `vmovq xmm,xmm` now swap to 0F 11 / 66-D6 when
   only that keeps VEX.B clear, reaching the 2-byte C5 form
   (`vmovss %xmm15,%xmm6,%xmm2` = `c5 4a 11 fa`, 4 bytes not 5). GAS's
   dir_encoding logic, byte-probed.
4. **[preference + hint, fixed] vpdpbusd/vpdpwssd default VEX in lccc but
   EVEX in GAS 2.47 (both x86-64 AND i686); vpmadd52* was misclassified
   EVEX-only (AVX-IFMA VEX rows exist); `{vex}`/`{vex2}`/`{vex3}` were
   accepted-and-ignored.** New `force_vex` plumbing: hint selectors are
   LAST-WINS (`{evex} {vex}` → VEX, GAS-probed); dual mnemonics take their
   VEX rows under the hint; EVEX-only shapes are rejected with GAS's exact
   message ordering (`unsupported instruction' for zmm/EGPR/high-vec,
   `unsupported masking' for masks, `unknown vector operation: `{1toN}'`
   for broadcasts, `no VEX/XOP encoding' for EVEX-only mnemonics and
   legacy `mov/add' under the hint). INT8 VNNI variants stay VEX (no EVEX
   form — verified).

Bonus fixes in the same area: EGPR **GPR-data** operands (not just memory)
now route to the EVEX arm (`vpinsrd $1,%r22d,...` was rejected; GAS
accepts; note vpinsrb/w with EGPR byte/word regs are rejected by BOTH —
kept), and `enclv` encoded 0F01E0 = SMSW %eax (now 0F01C0 per SDM).

## Verification
- `ci_local.sh --fast`: **74 gates green** (cargo-test 3503 passed after the
  OOM-safe recipe CARGO_PROFILE_FASTBUILD_DEBUG=0 CARGO_INCREMENTAL=0
  CI_LOCAL_JOBS=1 — the fastbuild-plain test compile OOM-kills on 4 GB).
- rustfmt + clippy: clean.
- encdiff casefile corpus: 11,645 instructions, lose=0 vs GAS 2.47,
  BEATS=124 (round-trip verified), net −151 bytes.
- isa_gap_sweep AFTER (x86-64): PASS 42462→42482, ENCDIFF 658→639,
  MISSING 19324→19323, FALSEACC=0. (i686): PASS +7, ENCDIFF −7, FALSEACC=0.
- New tests: `evex.casefile` (+70 lines: EGPR GPR-r/m matrix, r8-r15
  controls, vmovddup tuple lengths incl. boundary disp8s), `avx.casefile`
  (+23: direction matrix), new `vex-hint.casefile` (59 lines: defaults,
  {vex} forcing, 13 rejection classes, last-wins), 5 new unit-test fns in
  `evex_egpr_rm_tests` (all bytes GAS-probed), updated
  `vex_mem_source_forms` (it pinned the deliberate VNNI divergence with a
  comment admitting GAS defaults to EVEX — the audit data overturned that
  call; INT8 rows unchanged).

## Constraints / environment
No container swap possible (no root); 4.1 GB RAM host. The monolithic
lib-test compile OOM-kills without the debug=0 recipe above. GAS 2.47
provisioned from sourceware `releases/` (kernel.org mirror 404s the
`binutils-2.47.tar.xz` path).

## Next-agent backlog (ranked)
1. **AVX512-FP16 cluster** (~1,000+ MISSING lines: v*ph, vcvtph2*,
   vfmadd*ph...) — the largest single assembler-coverage gap on both suites.
   Correction (2026-09-26): the i7-14700KF target does **not** support
   AVX-512 or AVX512-FP16. This gap matters for other CPUs and ISA
   completeness, not for that machine's executable runtime performance.
2. Remaining WRONG-BYTES classes (36 lines, non-VEX): x87 bare `fadd`/
   `f{add,sub,mul,div}{,r}p %st(N)` operand-dropping (14), `.` position
   semantics + `.+N` expressions collapsing to 0 (10), relational
   immediates `$0 < 1` (6), `push +1`/`push 1 +` parsed as [0] (2).
3. LONGER quality gaps (~69): redundant %es prefix on string ops in 64-bit
   (cmpsb/insb/stosb/scasb), symbol-immediate width relaxation
   (`movb $xtrn,%al` → b0+R8; pushw $imm16 → 66 6a), tlscall ff10 relax,
   `lsl`/`{rex2} mov %al,%ah`/`{nf} imul` rows, vmov*.s AVX10.2 rows
   (encoding-choice ENCDIFF, both valid — decide GAS-parity vs keep).
4. TBM (~510 MISSING), vpexpandb/w, GFNI affine, SM4, AVX10.2 suffix
   converts (vcvtpd2psy etc.), vinsert/vextract f32x4/f64x2 shapes,
   `mov` high-reg forms, addr32/addr16.
5. `{vex}` ultra-corners with mixed widths + xmm16-31: GAS's
   position-dependent message order (`register type mismatch' vs
   `unsupported instruction') is not modeled; validate_operands fires
   first. Documented divergence, no real-world impact known.
6. Re-run `isa_gap_sweep --rank` after FP16 lands; the FP16 cluster alone
   moves MISSING by ~5% of the corpus.

## Red-team of THIS session's own work
- Two of my own first-draft unit-test expectations were wrong (vvvv
  inversion, an EVEX-vs-VEX r15 case); both caught by the GAS oracle and
  corrected — every byte in every test is oracle-probed, none from memory.
- The {vex} gate ordering was verified against 20+ GAS probes (zmm+mask,
  mask-only, high-vec src vs dst, EGPR, broadcast token echo, legacy
  stems, verr, XOP interplay, {evex}+{vex} last-wins).
- The beat-count drop 252→244 in the ENCDIFF battery is the
  previously-misencoded-shorter vmovddup/direction cases becoming correct
  ties — verified by the round-trip checker, not a regression.
