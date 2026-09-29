# Follow-up: VEX/EVEX encoding audit and semantic oracle hardening

**Date:** 2026-09-29
**Repository base:** `32299e7ff6522979d72f1476157a0ecb6b9eabf5`
**Upstream:** fetched `origin/main`; it still resolves to the same base.
**Scope:** i686 unsigned scalar conversions, x86-64/i686 assembler regressions, and the semantic trust boundary in `encdiff.py`/`insndiff.py`.

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
- 3,637 GAS-accepted forms rejected by LCCC (`REJECTS-VALID`); these are primarily in the already documented deferred ISA families (pinned convert spellings, newer FP16/BF16/AVX10 rows, VBMI2, GFNI/SM4, XOP, and related families). They remain gaps, not fixes, and were not silently counted as supported.
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

## Validation environment note

The first `scripts/ci_local.sh --fast` attempt on the S04 tree reported **96 passed, 7 failed, 5 skipped**. All seven failures shared one host prerequisite: the container lacked the i386 multilib C headers/runtime (`bits/libc-header-start.h`, `crti.o`, and `-lgcc` were unavailable under `-m32`). This affected the nocfi parity, reassociation, copy-alias, notype routing, i686 integer ISA, two-lane i64, and linker-suite gates; the linker suite reported 19 i386 fixture failures, not linker-oracle mismatches.

Installed Debian's `gcc-multilib`, `g++-multilib`, and `libc6-dev-i386` packages (matching the hosted CI prerequisites), then compiled and ran a minimal ELF32 executable with `gcc -m32`. Re-running each of those seven gates individually with `ci_local.sh --only` passed all seven; the linker suite then reported **301 pass, 0 fail, 0 warn, 0 skip**. This isolates the initial failures to missing host multilib, but these targeted reruns do **not** replace the final full-tree fast/slow gate.

The initial fast-run log is preserved at `/home/user/artifacts/ci_local-fast-2026-09-29.log` (SHA-256 `fe083c4be93108d012e471b897d260f1b4aa4340acad277ff255261627324fe1`). Individual recovered gate logs are under `/home/user/artifacts/ci-local-fast-recovered/`.

## Snapshot and final-gate policy

This branch has three validated checkpoint commits, all based on the current upstream main:

- `e1618a809079504849a6f58e0fe067a5b8ddadf1` — i686 `vcvtusi` rows.
- `8cffc3babd4f4d52783c865a0a869e0d5c4c8e10` — encdiff semantic validation and reject-group filtering.
- `45dee377353c46bd6aa46b070b84871ceae3a45c` — insndiff fail-closed decoding.

Snapshots S01–S03 were deliberately marked **UNGATED** because the full-tree CI mirror had not run yet; they are intermediate recovery points, not delivery snapshots. Before delivery, run `scripts/ci_local.sh --fast` followed by `scripts/ci_local.sh --slow` on the unchanged final tree, then publish the matching `mode=full` snapshot. The tree-matched pass stamp and `/home/user/artifacts/SNAPSHOT_LEDGER.md` are the authoritative CI/snapshot record. Do not edit the tree after that pass without rerunning the gate.

## Remaining work

1. Complete the exact-tree fast and slow CI passes, rustfmt, and clippy; record any failures and their causes before considering this deliverable final.
2. Triage the 3,637 shape-corpus `REJECTS-VALID` rows against the ranked deferred-family list in `FOLLOWUP-2026-09-27-evex-fp16-maps56.md`. Prioritize by emitted-code relevance and testability, not raw count. Each selected family still needs GAS 2.47 probes, compiler-oracle comparison where applicable, and positive/negative regressions.
3. The current encdiff/insndiff wrappers compare x86-64. Keep i686 coverage byte-exact through `asmdiff.py --32`; if multi-oracle i686 sweeps become a priority, add an explicit mode-aware wrapper rather than feeding 32-bit instructions through the x86-64 probe.
4. Do not claim runtime gains from these encoding-length results. The VM exposes two vCPUs and no usable PMU; no Raptor Lake performance measurement was obtained.
