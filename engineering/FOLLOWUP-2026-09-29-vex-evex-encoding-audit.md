# Follow-up: VEX/EVEX encoding audit and semantic oracle hardening

**Date:** 2026-09-30
**Repository base:** `02d4c9067651eb987881c41f907f639b28420db6`
**Upstream:** `origin/main` at `02d4c9067651eb987881c41f907f639b28420db6` after rebasing to the latest fetched main.
**Upstream changes:** PR #680 added `LCCC_SYSROOT` discovery (`324e270c8ed04e0906d81b730bf326864ba40dc5`); PR #679 merged the original encoder audit as `128bd082b74462f268670ffa37486c5f816ad887`. Those original fixes/tests are on current `origin/main`. This continuation is not doc-only: at its start local `main` was four commits ahead of the freshly fetched upstream, and the working tree added the substantive pinned-width VCVT implementation, VFPCLASS validation, i686 regressions, and restore-script multilib preflight described below.
**Scope:** the earlier i686 unsigned-scalar conversion and VEX/EVEX audit, the pinned-width packed-convert aliases and VFPCLASS validation, plus the September 30 correction to CI run `36640584448`: exact GAS 2.47 i686/x86-64 coverage and encoding support for the BF16 dot/pack, F32x2 broadcast, FP lane insert/extract, AVX-VNNI-INT16, and VSM4 forms exercised by the default i686 corpus.

## Continuation addendum: pinned-width converts and current ISA sources

This addendum supersedes the earlier statement below that only the follow-up document remained local. The current worktree adds an explicit GAS-alias parser and end-to-end encoder routing for packed-convert x/y/z width pins, along with x86-64 and i686 casefiles. The existing VFPCLASS implementation and VBMI2/i686 work are retained. A subsequent hosted-CI failure exposed five i686 corpus files for instruction families that the September 27 audit had explicitly deferred; rather than deleting or weakening those tests, the September 30 correction implements the tested forms and adds x86-64 counterparts. The exact failure and correction are recorded in the next section.

### CI run 36640584448: i686 differential root cause and correction

The supplied link targets job `109651639669`; unauthenticated access to the GitHub logs API returned HTTP 403, so the hosted log could not be read directly. The CI failure was reproduced by rebuilding the exact worktree and replaying its i686 assembly differential against GNU as 2.47.20260726. The local encoder run before this correction had passed the x86-64 differential; the broader hosted-job result is not inferred from the inaccessible log.

The pre-fix replay produced **666 passed, 6 failed (672 total)**. Every failure was `REJECTS-VALID`: five newly-added i686 casefiles exercised valid forms whose encoder families were explicitly still in the older follow-up's “Deferred work” list:

- BF16 `vdpbf16ps` / `vcvtne2ps2bf16`;
- EVEX `vbroadcastf32x2`;
- EVEX FP lane `vinsertf*` / `vextractf*`;
- VEX and EVEX AVX-VNNI-INT16 `vpdpwsud` / `vpdpwsuds` (two independent case groups);
- VEX.128/256 VSM4 `vsm4key4` / `vsm4rnds4`.

Thus the failure was a test/implementation scope mismatch in the in-flight change, not an i686 oracle or ELF32 differential bug. No tests were removed, skipped, or reclassified. The encoder now implements the covered forms using GAS 2.47-probed map/PP/opcode/tuple behavior; lane insert/extract additionally validate their legal width relationships and unsigned imm8, and the VEX VNNI-INT16 path rejects mixed vector widths. VSM4 remains VEX.128/256 only, as documented by Intel SDM v093; no EVEX extension was inferred. The VNNI-INT16 `vpdpwsud/s` forms use PP=F3 (not 66), W=0 and opcodes D2/D3 in both VEX and EVEX rows.

Red-team check: `vbroadcastf32x2` rejects an XMM destination, but `vbroadcasti32x2` accepts one under GAS 2.47 (`62 f2 7d 08 59 d1`). The two distinct instructions retain those different width rules; an initial shared-rule change was caught by the x86-64 corpus and reverted before final validation. The F32x2 memory form uses an 8-byte EVEX disp8 tuple.

The five new instruction-family casefiles now have both ELF32 and x86-64 versions. Final focused results against the pinned assembler are **672/672 i686** and **1386/1386 x86-64**, including all positive, negative, compressed-displacement, and mask cases. A focused Rust encoder unit test pins representative bytes and malformed-shape rejection; it passes. Rustfmt and Clippy pass. The fastbuild used `-O1`, two Cargo jobs, and the active 8 GiB swap file. Only these relevant fast gates were run; no slow gates were run locally, as requested. The checked-in architectural tables were cross-checked against Intel SDM v093 Vol. 2C (VPDPWSUD VEX rows, p. 5-491), Intel AVX10.2 Rev. 4.0 (the additional EVEX VNNI-INT16 rows, p. 174), and AMD APM Rev. 3.27 (VDPBF16PS p. 994, VBROADCASTF32X2 p. 831, VEXTRACTF32X4 p. 1002, and VINSERTF32X4 p. 1202); Intel SDM v093 documents the VSM4 VEX forms. These manuals establish architectural form/semantics. GAS-specific aliases, operand acceptance, and byte encodings remain grounded in GNU as 2.47 probes, not inferred from the manuals. Compiler corroboration harvested the same 70 positive instructions per mode: `encdiff.py` reports **70/70 `ok`** with GCC 16.2, Clang 23.1, GAS 2.47, LCCC, and the accepting rows of ICC 2021.10 / ICX; ICC and ICX each accepted 63/70 and had no byte mismatches among accepted rows. This is encoding comparison only, not a performance claim. Logs and byte probes are preserved at `/home/user/artifacts/ci_36640584448_{i686_asmdiff_after_fix,x86_64_asmdiff_after_fix,fastbuild_fix,fast_gates,python_fast_tests,i686_multioracle,x86_64_multioracle}.log` and `/home/user/artifacts/ci-i686-gas247-encoding-probes.txt`; the two per-mode compiler scoreboards are `ci_36640584448_{i686,x86_64}_multioracle.json` in the same directory (SHA-256: i686 `dc72af07c6615711bd64837a3ea6ef7e038d240321083da8a97caa9b7de2d34d`, x86-64 `18bfb78960970b7ee9ae4d46ca80424592272a5dce12a54ccb0bf919591f7f86`).

### Repository and shared-work check

The harness had removed `.git`. `scripts/arena_session_restore.sh` restored Git metadata by cloning `/home/user/artifacts/lccc.bundle`; it reported 190 preserved worktree modifications and did not reset or replace the worktree. A fresh fetch from `https://github.com/ms178/lccc.git` located `origin/main` at `02d4c9067651eb987881c41f907f639b28420db6`; the current branch's merge base is that exact commit. The rebase check is performed again before the final snapshot. The only open upstream PR observed during this pass was #682 (integer reductions, loop LICM, and oracle validation), unrelated to the x86 assembler changes.

### Architectural authority versus assembler syntax oracle

The implementation keeps these evidence roles separate:

- **Instruction semantics and architectural encoding forms:** Intel's current *Intel 64 and IA-32 Architectures Software Developer's Manual*, version 093, Volume 2C, together with the current Intel AVX10.2 Architecture Specification Rev. 4.0 where applicable; and AMD's latest available *AMD64 Architecture Programmer's Manual*, Volume 4, Rev. 3.27 (July 2026).
- **GAS mnemonic acceptance, x/y/z suffix spelling, forced `{vex}` selection, and emitted bytes:** GNU `as` **2.47.20260726** only. The suffixes are GAS-specific width selectors; they are not Intel/AMD architectural mnemonic suffixes. The implementation deliberately uses an explicit allowlist rather than inferring that every trailing x/y/z is a legal convert alias.
- **Independent assembler/compiler byte cross-checks:** GCC 16.2 (`cg162`), Clang 23.1 (`cclang2310`), ICX (`cicxlatest`, the moving alias at run time), and ICC 2021.10 (`cicc2021100`) through `encdiff.py`. These comparisons use the canonical architectural mnemonic, not the GAS-only pinned alias spelling.

Primary references consulted and retained under `/home/user/artifacts/`:

1. Intel, *Intel 64 and IA-32 Architectures Software Developer's Manual*, Vol. 2C, **v093**, official catalog updated 2026-09-21; current PDF: <https://cdrdv2-public.intel.com/929355/326018-093-sdm-vol-2c.pdf>. Relevant printed pages: VCVTDQ2PH 5-30–5-31; VCVTNEPS2BF16 5-38–5-39; VCVTPD2PH 5-40–5-41; VCVTQQ2PH 5-83–5-84; VFPCLASSPH 5-348–5-350 and VFPCLASSPS 5-351–5-352. The exact downloaded PDF is `/home/user/artifacts/intel-sdm-v093-vol2c.pdf` (SHA-256 `86651d693340a6fedb9851e459945eafba83a877d4bbe2e4575fcfdb531bb68`).
2. Intel, *Intel AVX10.2 Architecture Specification*, **Rev. 4.0**, May 2025, document 361050-004US: <https://cdrdv2-public.intel.com/828965/361050-intel-avx10.2-spec.pdf>. Section 7.6, pp. 105–107, specifies VFPCLASSBF16's EVEX map/pp/opcode, packed widths, immediate classification and operation. It is the primary source for the AVX10.2 BF16 row; the current SDM Vol. 2C covers the FP16 VFPCLASS rows.
3. AMD, *AMD64 Architecture Programmer's Manual*, Vol. 4, **Rev. 3.27**, July 2026, publication 26568: <https://docs.amd.com/v/u/en-US/26568_3.27_APM_Vol4_PUB>. Relevant pages: VCVTDQ2PH 860–861; VCVTNEPS2BF16 874–875; VCVTPD2PH 876–877; VCVTQQ2PH 915–916; VFPCLASSPH 1143–1144; VFPCLASSPS 1146–1147; VFPCLASSSH 1145. The extracted current manual is `/home/user/artifacts/amd-26568-rev3.27-2026.txt` (SHA-256 `9040a362cabb0de5d90be52ad2222415a1221de97304f4bea9353b1b2c697769`).
4. A compact extraction of the cited Intel SDM/AVX10.2 pages and AMD instruction entries is `/home/user/artifacts/assembler-current-doc-excerpts-2026-09-29.txt` (SHA-256 `92e8963f5278bbc53c88ac9ae09cabf7770fcc1c9417c3c81dfc02daa9b174f1`).

### Implemented pinned-convert behavior

`vcvt_params()` now resolves exact native instruction names before attempting any suffix parse (important for native names such as `vcvtps2phx` and `vcvtph2psx`). Its suffix grammar is a GAS 2.47-probed allowlist. The pin is carried through shape validation, VEX.L or EVEX.L'L, memory broadcast-count validation, EVEX tuple displacement scaling and mode routing. Width-mismatched registers, ambiguous unsuffixed narrow-memory forms, mismatched broadcasts, unsupported z spellings, and spurious extra suffixes remain rejects.

The covered aliases are:

- VEX families: `vcvtpd2ps{x,y}`, `vcvtpd2dq{x,y}`, `vcvttpd2dq{x,y}`.
- EVEX AVX512-DQ/F forms: `vcvtpd2udq{x,y}`, `vcvttpd2udq{x,y}`, `vcvtqq2ps{x,y}`, `vcvtuqq2ps{x,y}`.
- EVEX 2:1 conversions: `vcvtdq2ph{x,y}`, `vcvtudq2ph{x,y}`, `vcvtneps2bf16{x,y}`, and the aliases `vcvtps2phxx` / `vcvtps2phxy` whose native base mnemonic is `vcvtps2phx`.
- EVEX 4:1 conversions: `vcvtpd2ph{x,y,z}`, `vcvtqq2ph{x,y,z}`, and `vcvtuqq2ph{x,y,z}`.

`vcvtneps2bf16` is a real architectural dual-encoding case: Intel SDM v093 lists VEX.128/VEX.256 as well as EVEX.128/.256/.512. GAS 2.47 defaults the unsuffixed form to EVEX and accepts `{vex}` to select map 2 / 0F38; the x/y pins select their source widths in either row. The implementation preserves that default preference while supporting explicit VEX selection. Its full-vector memory tuple scaling is regression-tested at the signed disp8 boundary: x/16-byte and y/32-byte rows with displacements 2032/4064 both encode disp8 `7f` in x86-64 and i686.

### Validation results for this continuation

- `scripts/build_lccc_fast.sh` rebuilt the final code at `-O1`, using the required two build jobs; the focused Rust test `vcvt_pinned_x_y_z_aliases_and_vex_dual_form` passes (**1 passed, 0 failed**, 3880 filtered).
- GAS 2.47.20260726 whole-object differentials: pinned VCVT casefile **12/12** in x86-64 and **12/12** in i686; existing VFPCLASS casefile **19/19** in x86-64 and **13/13** in i686. `asmdiff` counts case groups, including the independent reject groups.
- `encdiff.py` canonical-convert corpus: **10/10 `ok`** in each mode, comparing LCCC, GAS 2.47, GCC 16.2, Clang 23.1, ICX and ICC. Every row is byte-identical across all five assemblers in x86-64 and i686; there are no `BEATS`/performance claims.
- `encdiff.py` VFPCLASS corpus: 4 canonical forms per mode. GAS, GCC and Clang accept the FP16/FP32/FP64 and AVX10.2 BF16 rows and match LCCC bytes. ICC and ICX accept the three FP16/FP32/FP64 rows but do not accept the newer `vfpclassbf16` AVX10.2 row; this is recorded as compiler-oracle coverage, not treated as evidence against the current Intel specification. No accepted-row byte mismatches were found.
- After the successful fastbuild, the canonical-oracle sweep was rerun from the current `target/fastbuild/lccc`: 10 pinned VCVT rows in each mode and 4 VFPCLASS rows in each mode. All VCVT bytes matched GAS/GCC/Clang/ICC/ICX; all VFPCLASS rows matched every accepting oracle, while ICC and ICX continue to reject only the AVX10.2 `vfpclassbf16` form. The archived rows were byte/value-compared to the rerun scoreboards. Reproduction log: `/home/user/artifacts/encdiff-current-vcvt-vfpclass-2026-09-29.log` (SHA-256 `a3918f39bfa8cfcb764909aa9c99c54e0ef4032ca7b2e095ce684655f35ad65d`). The four current scoreboards are retained at `/home/user/artifacts/pinned-vcvt-multioracle-{x86_64,i686}-2026-09-29.json` and `/home/user/artifacts/vfpclass-multioracle-{x86_64,i686}-2026-09-29.json`; their hashes are listed below.
- The first full `ci_local.sh --fast` preflight reached **102 passed, 1 failed, 5 skipped**. The sole failure was the i386 linker-suite C++ DSO fixture: the image had gcc/libc i386 multilib but lacked `g++-multilib`, producing `/usr/include/c++/14/exception:35:10: fatal error: bits/c++config.h: No such file or directory`. No gate was weakened. Installed Debian `g++-multilib` / `lib32stdc++-14-dev`; a direct `g++ -m32` `<stdexcept>` compile now passes, and the linker suite rerun reports **301 pass, 0 fail, 0 warn, 0 skip**. `scripts/arena_session_restore.sh` now preflights and provisions this C++ multilib dependency so a future harness restore does not repeat the failure.
- After provisioning, the older tree's `ci_local.sh --fast` passed **103 passed, 0 failed, 5 skipped** on tree `1f5f1d50b3d176a82c06ce465b66ac4d41c91b47`: x86-64 GAS 2.47 differential **1350/1350**, i686 **636/636**, linker suite **301/0/0/0**, rustfmt and Clippy passed. Log: `/home/user/artifacts/ci_local-fast-pinned-vcvt-2026-09-29.log` (SHA-256 `80327e7eea74227e5315e4769ccb2ea02952ca76d0d634f9ddd5bd9685f1e8d7`). This is an historical fast-only stamp for a different tree; the September 30 instruction-family correction has its own targeted validation above. Do not treat this as a full-tree stamp or infer that local slow gates were run; slow gates are left to hosted GitHub CI.

This work validates encodings and syntax/shape correctness. It does not claim generated-code speedups; no hardware PMU is available in this environment, and no runtime performance measurement was made.

## Landed changes

### 1. i686 unsigned integer-to-scalar EVEX conversions

GAS 2.47 accepts `vcvtusi2ss`/`vcvtusi2sd` in 32-bit mode both unsuffixed and with an explicit `l` source-width suffix, with either a low 32-bit GPR or memory source. LCCC previously rejected these because the i686 vector-delegation guard deliberately kept GP-data instructions out of the shared x86 encoder, while the local i686 dispatch had no unsigned EVEX arms.

The i686 encoder now has a narrow exception for those four W=0 spellings and delegates them to the shared encoder. The exception does not open the boundary for unrelated GP operands or unavailable registers. The `q` spellings remain 64-bit-only and return GAS's mode diagnostic rather than letting the x86-64 core encode a W=1 row in `.code32`.

Pinned GAS 2.47 bytes:

| Form | Register source | Memory source |
|---|---|---|
| `vcvtusi2ss` / `vcvtusi2ssl` | `62 f1 76 08 7b d0` | `62 f1 76 08 7b 10` |
| `vcvtusi2sd` / `vcvtusi2sdl` | `62 f1 77 08 7b d0` | `62 f1 77 08 7b 10` |

The new `tests/asm-diff/i686/evex-unsigned-scalar-conversions.casefile` covers all register/memory and suffix forms, upper-case spelling, forced EVEX, and independent rejection groups for `q`, forced VEX, masking, and a YMM operand. The accompanying Rust tests pin the bytes and the mode/VEX diagnostics. The x86-64 VEX-error formatter now also strips only the `l`/`q` source-width suffixes of these conversions, matching GAS without stripping real suffix letters from other EVEX mnemonics.

**Targeted result:** the full i686 assembler differential against GAS 2.47 passed **608/608** cases after this change.

### 2. `encdiff.py` now fails closed on semantic equivalence

The old classifier had two shorter-than-best paths that assigned `BEATS` without invoking `decodes_same()`. It could therefore report a semantic win from byte length alone. The classifier has been changed so that:

- A shorter candidate must round-trip against every distinct shortest oracle encoding before it can be `BEATS`.
- Same-length byte differences and longer-than-best candidates are also round-trip checked before being called correct or merely longer.
- `decodes_same()` distinguishes a real disassembly mismatch (`False`) from missing/failed/unsupported disassembly (`None`). Empty byte streams, objdump failures, `.byte` output, and `(bad)` output cannot count as successful equivalence.
- A mismatch becomes `WRONG-BYTES`; unavailable evidence becomes `UNVERIFIED-BEATS` or `UNVERIFIED-BYTES`. Both unverified classes fail the command, as do incorrect bytes and unaccounted longer encodings.
- The size summary now says `shorter/tie/longer`, explicitly labels itself raw byte-length data, and the JSON schema is 3. These counters are not semantic verdicts.
- `read_casefiles()` skips whole `reject` groups. This avoids intentionally invalid instructions poisoning and recursively splitting remote batches.

The GAS disassembler marks a legal VEX encoding of some EVEX-only mnemonics with the GNU pseudo-prefix `{vex}` (for example, `{vex} vpdpbusds`). That prefix describes the encoding selector, not a different architectural operation. The canonicalizer now removes it before comparison. The behavior is covered by `scripts/test_encdiff.py`, which also tests both former `BEATS` paths, mismatch and unavailable-disassembler cases, invalid bytes, equal-length disagreement, longer candidates, and reject-group filtering. The test is wired into both `scripts/ci_local.sh` and hosted CI.

### 3. `insndiff.py` round-trip hardening

A targeted `insndiff.py` run initially left the two legal VEX encodings of `vpdpbusds` as unverified `SHORTER` rows because its disassembler canonicalizer retained objdump's `{vex}` marker. Its verifier also treated two empty/undecodable disassembly results as equal. It now strips the pseudo-prefix and fails closed on empty bytes, nonzero objdump exit, missing decode output, `.byte`, or `(bad)`. The shared offline test pins both behaviors.

## Oracle and corpus results

### Toolchain identity

- GNU assembler/objdump: Binutils **2.47.20260726**.
- Compiler Explorer alias audit on 2026-09-29: GCC `cg162` **16.2**, Clang `cclang2310` **23.1.0**, ICC `cicc2021100` **2021.10.0**, and ICX `cicxlatest` (the moving latest alias). All pinned aliases were current; all five encdiff oracles were reached in the multi-oracle runs.

### Curated six-casefile VEX/EVEX run

The positive groups from `avx`, `evex`, `evex-avx512f-dq-bw`, `fp16-evex`, `vaes`, and `xop-vpermil2` yielded **1,860 unique instructions** after reject-group filtering. The five-oracle result was:

| Verdict | Count |
|---|---:|
| `BEATS` (round-trip verified against all shortest forms) | 2 |
| `ok-best` | 135 |
| `ok` | 1,703 |
| `DECLINED-FP` | 20 |
| `WRONG-BYTES`, `UNVERIFIED-*`, `REJECTS-VALID`, `LONGER` | 0 |

The two verified `BEATS` are the memory-source `vpdpbusds` XMM and YMM forms. LCCC emits a legal VEX encoding one byte shorter than each oracle's default EVEX encoding:

- `vpdpbusds (%rax), %xmm1, %xmm2`: LCCC `c4e2715110` (5 bytes), oracle `62f275085110` (6 bytes).
- `vpdpbusds 0x40(%rdi,%rsi,8), %ymm3, %ymm4`: LCCC `c4e2655164f740` (7 bytes), oracle `62f265285164f702` (8 bytes).

Objdump's `{vex}` marker now normalizes correctly, and both rows were confirmed equivalent against every shortest oracle form. They are already covered by the `vnni_mem betterok` group in `tests/asm-diff/avx.casefile`; this audit did not claim a new hardware speedup.

All 20 `DECLINED-FP` rows are packed/scalar FP add or multiply source swaps that Clang/ICX use to reach a shorter prefix. LCCC deliberately preserves source order because swapping can change SRC1 NaN-payload propagation. The raw per-oracle length totals are in the JSON artifact; they are not performance measurements. ICC accepted 1,853 of 1,860 rows and ICX 1,679; GAS, GCC, and Clang accepted all 1,860.

A separate `insndiff.py` probe set now reports `BETTER=3`: the two `vpdpbusds` rows above plus `vpand %ymm9, %ymm0, %ymm2` (shorter than GAS and equal in length to Clang/ICX, so not a multi-oracle `BEATS`).

### Binutils 2.47 shape corpus

`vec_opt.py` extracted 53,882 vector lines from 934 Binutils testsuite files, deduplicated them into 25,983 shape keys, and retained **25,586 GAS-64-accepted representatives** spanning 1,178 mnemonics. The offline LCCC/GAS 2.47 screen reported:

- 21,949 accepted by both LCCC and GAS: **21,857 `ok` + 92 round-trip-verified `BEATS`** versus GAS.
- 3,637 GAS-accepted forms rejected by LCCC (`REJECTS-VALID`) in that **pre-continuation** screen. This is not a current-tree count: the screen predates the pinned-width VCVT and VFPCLASS work documented above. It included deferred FP16/BF16/AVX10, VBMI2, GFNI/SM4, XOP and related forms; the exact remaining set must be re-screened before assigning a current gap count.
- No `WRONG-BYTES` or `UNVERIFIED-*` rows among accepted encodings after semantic validation.

The 92 GAS-shorter candidates were then sent to all five compiler oracles. Every row was `ok-best`: Clang and ICX matched LCCC's shorter lengths, while GAS/GCC/ICC used encodings one byte longer. Thus none is a strict win over the best compiler oracle. This is a size/encoding comparison only; no runtime or hardware-performance conclusion follows.

## Reproducible evidence

The generated corpora and full JSON scoreboards are preserved under `/home/user/artifacts/` so they survive harness resets. SHA-256 digests:

| Artifact | SHA-256 |
|---|---|
| `vex-evex-curated-corpus-2026-09-29.txt` | `c4d434266c4328843233522a53dcc166d07d619339968a35f8dea1e473d73970` |
| `vex-evex-curated-multioracle-encdiff-2026-09-29.json` | `cfa6aec78b3a144378b2189df94e8dd1717fe10da60d7cae859982ea7b539eba` |
| `vex-evex-insndiff-probes-2026-09-29.txt` | `85bd0eb51ea58064523e61f72dafb5fde1742c908f6007d00f96dbd85d77f9d5` |
| `vex-evex-shape-corpus-gas247-2026-09-29.txt` | `432f904d9d3b7d61bde65f92db45471fdf373689994c7df29113e7981edd63a5` |
| `vex-evex-shape-gas-shorter-multioracle-encdiff-2026-09-29.json` | `64438de45ff83e1d07a3d1acae3961d070ec746c4bc17f61d2fba460a2d770aa` |
| `vex-evex-shape-offline-encdiff-2026-09-29.json` | `3013587f17d4e5373c4f9676ff8aa2e072f0354ddf4aa4e5a172ef6322718b07` |

The corpus generator and invocation are documented in `scripts/vec_opt.py`; the curated run can be replayed directly with `scripts/encdiff.py --casefiles ...` and the six casefiles above.

### Current continuation scoreboards and source files

The per-instruction compiler-oracle byte records and current-manual extracts are durable under `/home/user/artifacts/`. Hashes:

| Artifact | SHA-256 |
|---|---|
| `pinned-vcvt-multioracle-x86_64-2026-09-29.json` | `4f3909f771c231e1d941f944dd327a9bff855315fb009e2636814a5e9cce0eb1` |
| `pinned-vcvt-multioracle-i686-2026-09-29.json` | `2bbf56d02f08d523e76c26ba606c1ac81bb9448f050f34216a463f389c2597f9` |
| `vfpclass-multioracle-x86_64-2026-09-29.json` | `6a9fbee63c267295d4dbbd99f7a17c17f906d91318e59c11654a73d31e55d480` |
| `vfpclass-multioracle-i686-2026-09-29.json` | `9e31f403e7d1a3ddc63495e852f0f44337bf7e68cfda71deb32d8ccbc1b332a7` |
| `pinned-bf16-disp-probes-x86_64.json` | `92c810a01f393c47d6f86638295f06b0ed545b7f94818b5739fc5196d9b62fdf` |
| `pinned-bf16-disp-probes-i686.json` | `7ddacf4ebde443fb52277a7a859bdaf14eb04792d4e337cb1a94084d1d02f978` |
| `intel-sdm-v093-vol2c.pdf` | `86651d693340a6fedb9851e459945eafba83a877d4bbe2e4575fcfdb531bb68` |
| `intel-avx10.2-2025-rev4.pdf` | `25fe4fd43d5e5a2661e02666c06cc14e0b08efb812d48d7fb29abe9566e21910` |
| `amd-26568-rev3.27-2026.txt` | `9040a362cabb0de5d90be52ad2222415a1221de97304f4bea9353b1b2c697769` |
| `assembler-current-doc-excerpts-2026-09-29.txt` | `92e8963f5278bbc53c88ac9ae09cabf7770fcc1c9417c3c81dfc02daa9b174f1` |
| `ci_local-fast-pinned-vcvt-preflight-2026-09-29.log` | `3db56e9eac2cb746f66c28b78c78787f92d49dfcf2cdacd338590008d49178ae` |
| `ci_local-fast-pinned-vcvt-2026-09-29.log` | `80327e7eea74227e5315e4769ccb2ea02952ca76d0d634f9ddd5bd9685f1e8d7` |
| `encdiff-current-vcvt-vfpclass-2026-09-29.log` | `a3918f39bfa8cfcb764909aa9c99c54e0ef4032ca7b2e095ce684655f35ad65d` |
| `pinned-vcvt-oracle-input-x86_64-2026-09-29.txt` | `fe2d0bc1f1ebfe16bd69fc15e9229645f74039394eb4eebe10fcd7549b6fd41b` |
| `vfpclass-oracle-input-x86_64-2026-09-29.txt` | `1f417c4ba435790a4eb7c7a289c5ac363c549f2b287a678634fafe76e7da3a71` |

The multi-oracle scoreboards are regenerated with `scripts/encdiff.py --lccc target/fastbuild/lccc --file <input.txt> --json <scoreboard.json>`; add `--32` for i686. Canonical instruction inputs are saved as `/home/user/artifacts/pinned-vcvt-oracle-input-{x86_64,i686}-2026-09-29.txt` and `/home/user/artifacts/vfpclass-oracle-input-{x86_64,i686}-2026-09-29.txt`. `--32` selects LCCC/GAS 32-bit mode plus remote compiler `-m32`. The x/y/z spellings themselves are compared directly to GAS in the committed casefiles, because those spellings are GAS syntax rather than architectural names.

## Validation environment note

The first `scripts/ci_local.sh --fast` attempt on the S04 tree reported **96 passed, 7 failed, 5 skipped**. All seven failures shared one host prerequisite: the container lacked the i386 multilib C headers/runtime (`bits/libc-header-start.h`, `crti.o`, and `-lgcc` were unavailable under `-m32`). This affected the nocfi parity, reassociation, copy-alias, notype routing, i686 integer ISA, two-lane i64, and linker-suite gates; the linker suite reported 19 i386 fixture failures, not linker-oracle mismatches.

Installed Debian's `gcc-multilib`, `g++-multilib`, and `libc6-dev-i386` packages (matching the hosted CI prerequisites), then compiled and ran a minimal ELF32 executable with `gcc -m32`. Re-running each of those seven historical gates individually with `ci_local.sh --only` passed all seven; the linker suite then reported **301 pass, 0 fail, 0 warn, 0 skip**. This isolates that earlier batch of failures to missing host multilib; it is separate from the September 30 assembler-corpus failure and its targeted validation above.

The initial fast-run log is preserved at `/home/user/artifacts/ci_local-fast-2026-09-29.log` (SHA-256 `fe083c4be93108d012e471b897d260f1b4aa4340acad277ff255261627324fe1`). Individual recovered gate logs are under `/home/user/artifacts/ci-local-fast-recovered/`.

## Snapshot and final-gate policy

Historical baseline: the original encoder audit, regression tests and CI wiring landed in PR #679 (`128bd082b74462f268670ffa37486c5f816ad887`) at `02d4c9067651eb987881c41f907f639b28420db6`; rebase found those changes already present and dropped the duplicates. This continuation is **not** the old doc-only delta: at its start the local branch had four commits above the freshly fetched `origin/main` (`bd89fe1d`, `ec036f09`, `e8d46582`, `158cea53`) plus the VCVT/VFPCLASS and CI-correction work described in the addenda. The final patch base is rechecked against the latest `origin/main` at delivery. The S09 full-CI stamp applies only to its old tree; S10 VBMI2 was explicitly an UNGATED interim snapshot. Neither historical stamp substitutes for validation of the current tree.

Snapshots S01–S07 in the existing ledger are historical **UNGATED** checkpoints: S01–S06 were built against `32299e7...`, and S07 was based on `17479e69...`; neither base is current. They are recovery/history points, not delivery patches. Any checkpoint made before matching checks on the exact candidate tree is likewise intermediate. The current task runs only the relevant local fast gates; do not run local slow gates, which are delegated to hosted GitHub CI. Keep the final patch, tree hash, and corresponding fast-gate logs together in `/home/user/artifacts/SNAPSHOT_LEDGER.md`; do not claim a local full/slow stamp.

## Remaining work

1. **Hosted/full-tree validation:** the September 30 delivery ran the relevant fast checks only: pinned GAS 2.47 differentials and the 70-row GCC/Clang/ICX/ICC compiler comparison in both modes, the focused encoder unit, `scripts/test_encdiff.py`, rustfmt, and Clippy. This is not a full-tree `ci_local.sh --fast` stamp. Do not run local slow gates; rely on GitHub CI for slow and full-workflow coverage. If a future task requests a full local fast-gate stamp, record it against the exact unchanged tree in `/home/user/artifacts/SNAPSHOT_LEDGER.md`.
2. The **3,637** shape-corpus `REJECTS-VALID` count above is a historical pre-continuation screen, not a current remaining-gap count: the corpus has not yet been rerun after pinned-convert and VFPCLASS additions. Re-screen the saved 25,586-representative corpus before quoting a new number, then triage remaining families by emitted-code relevance and testability rather than raw count.
3. `encdiff.py --32` already provides mode-aware LCCC/GAS `--32` and remote compiler `-m32` comparisons; this continuation used it on canonical VCVT and VFPCLASS forms. The full multi-oracle VEX/EVEX corpus recorded earlier was x86-64; there is no claim that the entire 1,860-instruction corpus was re-run in i686 mode. Keep GAS 2.47 x86-64/i686 casefiles as the syntax/byte-acceptance gate.
4. Compiler-version coverage for the AVX10.2-only `vfpclassbf16` row is intentionally partial: GCC 16.2 and Clang 23.1 accept it; ICC 2021.10 and the `cicxlatest` alias did not. Preserve that limitation in future evidence rather than treating unsupported compiler parsers as architectural counter-evidence.
5. Do not claim runtime gains from encoding-length results. The VM exposes two vCPUs and no usable PMU; no Raptor Lake performance measurement was obtained.
