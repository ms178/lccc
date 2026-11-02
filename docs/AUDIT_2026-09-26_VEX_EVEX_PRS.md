# LCCC Red-Team Audit — PRs #623/#624/#627 (VEX/EVEX area) + empirics

**Auditor:** session agent (2026-09-26)
**Base:** e5bc191 (ms178/lccc main, includes PR #627 = 413bdf7)
**Oracles:** GAS 2.47.20260726 (local, x86_64+i686), godbolt API (clang 23.x, gcc 16.x?, icc, icx), objdump 2.44 + binutils 2.47 source.
**Method:** encdiff.py (round-trip verified, all 5 oracles) over the ENCDIFF subset of the GAS-2.47 testsuite sweep (658 lines, x86-64); isa_gap_sweep.py x86-64 (122,508 lines: PASS=42462 ENCDIFF=658 MISSING=19324 FALSEACC=0 SKIP=60064); targeted GAS byte-probes; binutils source cross-reading (i386-dis.c OP_EX/print_register, tc-i386.c build_evex_prefix/build_apx_evex_prefix).

## Verdict on the merged PRs (agree/disagree)

### PR #627 (413bdf7, EVEX decorators/promoted rows/symimm) — mostly AGREE, 3 defects found
AGREE (verified correct by probes):
- Decorator duplicate detection, {k0} rejection, standalone {sae} handling — spot-checked vs GAS 2.47: consistent.
- vpextrw C5-row field order (GP in reg, xmm in rm): byte-probed `{evex} vpextrw $1,%xmm16,%r9d` = `62 31 7d 08 c5 c8 01` ✓ matches GAS.
- vmpsadbw AVX10.2 EVEX row (F3.0F3A.W0 42, N=VL): matches GAS for ymm/zmm + masks.
- Symbol-immediate sized forms for memory r/m: verified 80/81 + R8/R16/R32/R32S shapes.
- Scalar {1toN} rejection classes: spot-checked vaddss/vmovss/vcvtsd2ss families — messages match GAS.

DISAGREE (defects, all confirmed by oracle disassembly round-trip):
1. **[CRITICAL — miscode] GPR r/m ≥16 in EVEX mod=3 is mis-encoded.**
   `vcvtsi2sdq %r21, %xmm29, %xmm30` → lccc `62 21 97 00 2a f5` decodes to
   `vcvtsi2sd %rbp,...` (r21 became rbp!). `vextractps $1,%xmm16,%r20d` → decodes to
   `%esp` instead of `%r20d`. Root cause: `emit_evex_mod3` (avx.rs ~484) uses the
   VECTOR r/m +16 bit (EVEX.X = P0.b6, inverted) for GPR operands. Hardware/binutils
   semantics: vector r/m +16 ← X (P0.b6, inverted); GPR (EGPR) r/m +16 ← rex2.B
   (P0.b3, SET, NON-inverted) with X inactive. The memory path (emit_evex_memop →
   evex_addr_bits/apply_evex_apx_addr) already implements the GPR model — the mod=3
   path was forgotten. All 5 oracles agree; GAS build_apx_evex_prefix source confirms.
2. **[CRITICAL — miscode] vmovddup EVEX memory tuple is N=16/32/64 (full VL) but must be Tuple1 N=8.**
   `vmovddup -1024(%rdx), %xmm30` → lccc disp8=0xc0 (-64×16=-512: WRONG ADDRESS),
   GAS disp8=0x80 (-128×8=-1024 ✓). encode_evex_unary computes N=VL/tuple_div with
   tuple_div=1 for opcode 0x12; the PR's own evex_scalar_tuple_n table (avx.rs 2704)
   correctly classifies vmovddup (pp=3, 0x12) as Tuple1 N=8 — but only the decorator
   rejection consumes it, not the displacement encoder. The table and the encoder
   disagree with each other INSIDE the same PR.
3. **[QUALITY] vmovss/vmovsd 3-op and vmovq xmm,xmm never use the mirrored opcode
   to reach the 3-byte VEX form.** `vmovss %xmm15,%xmm6,%xmm2`: GAS `c5 4a 11 fa`
   (0F 11, rm=dst) vs lccc `c4 c1 4a 10 d7` (0F 10, rm=src2 → B=1 → C4, 5 bytes).
   `vmovq %xmm15,%xmm6`: GAS `c5 79 d6 fe` (66/D6 row) vs lccc C4+7E. GAS's
   dir_encoding (tc-i386.c) picks the direction whose r/m needs no B extension.
4. **[QUALITY] vpdpbusd/vpdpwssd encoding preference is VEX in lccc, EVEX in GAS.**
   These are the only two VNNI mnemonics with BOTH AVX-VNNI (VEX) and AVX512-VNNI
   (EVEX) forms; GAS 2.47 always emits EVEX (also when VEX is same length or longer).
   INT8 variants (vpdpbssd/bsud/wssud) are VEX-only and already agree.

### PR #624 (cde38d5, {evex} promotion) — AGREE with design; the promoted-row GPR bug above lands here too
The {evex} dispatch-before-soundness-gate design is correct and the promoted.rs row
data I probed (vpextrw C5, vinsertps, vmovd/q LIG) matches GAS. But every promoted
row that puts a GPR in r/m inherits defect #1 (vcvtsi2sdq/vextractps/vpextrd/vpextrq/
vpinsrd/vpinsrq/vmovd/vmovq-to-GPR with r16-r31). The PR's own test battery only used
low GPRs (r8-r15 exercise B3 but not B4) — a test-coverage blind spot, not a design flaw.

### PR #623 (c1c6736) / 3b0c672 — no NEW defects attributable
Their areas (ISA state handling, HLE, GFNI, EVEX shifts/converts) came out of the
sweep FALSEACC=0 and no WRONG-BYTES in those mnemonic families.

## Other WRONG-BYTES classes found (outside the audited PRs, real bugs)
- `enclv` encodes as 0F01E0 = SMSW %EAX (must be 0F01C0). One-byte opcode-table bug.
- x87 bare `fadd`/`fmul` and all `f{add,sub,mul,div}{,r}p %st(N)` one/two-operand
  forms ignore the st(N) operand (emit DE C1-class default). 14 lines.
- `.` (current-location symbol) resolves to post-instruction in lccc, pre-instruction
  in GAS (jrcxz . → e300 vs e3fe; call . → disp 0 vs -5). 10 lines.
- `.+N` / `target-.` expressions collapse to displacement 0 (ja .+0x1234, movq $(xtrn - .), %rax).
- GAS relational operators in immediates ($0 < 1 etc.) evaluate to 0 instead of 1. 6 lines.
- `push +1` / `push 1 +` parsed as memory [0] instead of immediate. 2 lines.
- String ops with explicit %es emit a redundant 26 prefix (cmpsb/insb/stosb/scasb). ~10 lines.
- Symbol immediates to registers always take imm32 (movb $xtrn,%al must be B0+R8; pushw $imm must be 66 6A for -32768..32767 range...GAS byte-probed). ~20 lines.
- tlscall indirect call relaxation: ff 90 00000000 → ff 10. 3 lines.

## Sweep inventory (x86-64, for the follow-up doc)
MISSING clusters by family: AVX512-FP16 (~1000+ lines: v*ph, vcvtph2*, vfmadd*ph...),
TBM (~510), AVX10.2 suffix converts (vcvtpd2psy/vcvtneps2bf16x .s/.y forms, ~80),
vinsert/vextract f32x4/f64x2 memory/high-reg shapes (~175), vpexpandb/w (~76),
GFNI affine (~72), SM4 (~84), mov high-reg forms (~276), addr32 (~72).
FALSEACC=0 across the whole corpus — no soundness holes. lccc BEATS all oracles on
192/658 ENCDIFF lines (round-trip verified) and is net -202 bytes vs GAS on that set.
