# Merged PR #629: independent red-team audit and follow-up

**Date:** 2026-09-26. **Base:** upstream `ms178/lccc` `main` at
`9c806fe274833d9d1268eb7506d91e7bc7372965` (merge of PR #629).
`git ls-remote origin refs/heads/main` was checked after implementation and
still returned that SHA. This is a follow-up to the merged work, **not** a
reapplication of `ms178-1.patch`. No prior worktree's uncommitted state was
destroyed. The working hardware is a two-vCPU Xeon VM with approximately
1.9 GiB RAM and an active 8 GiB swap file, **not** the i7-14700KF target.

## Method and evidence

- Assembled each line independently with GNU **as 2.47.20260726**, once
  for x86-64 and/or i686, and compared either `.text` bytes or a real
  assembler rejection against **unchanged merged main** and the candidate.
  Inputs/results: `/home/user/pr629-evidence/probes/baseline-probes.json`,
  `extended-probes.json`, and `final-probes.json` (88 focused cases in the
  combined original/extended set). All three files retain the actual
  diagnostics, not hand-decoded assumptions. An additional **180-input**
  broadcast source-width/length matrix and a **48-input** EVEX-only-table
  probe of twelve low-register forms, with and without `{vex}`, were run
  against both GAS targets. The independent oracle assemblers and baseline
  compiler were preserved under `/home/user/.cache/gas-2.47-*/bin/as` and
  `/home/user/pr629-evidence/baseline/lccc`.
- Added one-instruction regression groups (90 x86-64, 63 i686), including
  legal bytes **and** isolated rejects. GAS 2.47 whole-object differentials:
  **1,045/1,045 x86-64** and **91/91 i686** pass, including the existing
  cases; the installed GAS 2.44 also passes the new x86-64 groups (90/90)
  and full i686 corpus (91/91). The runners compare relocations and symbols
  as well as instruction bytes for accepted inputs. Casefiles:
  `tests/asm-diff/merged-pr629-followup.casefile` and
  `tests/asm-diff/i686/merged-pr629-followup.casefile`.
- New Rust unit tests pin exact GAS-probed VEX2/C4, VEX3/C4, XOP,
  low-register EVEX, GPR-width, APX address-extension and i686 bytes as well
  as representative exact diagnostics. `scripts/ci_local.sh --fast` is
  **green: 75 passed, 0 failed, 3 intentionally skipped slow gates**,
  including rustfmt, Clippy and 3,508 passing library tests (7 ignored).
  Full log: `/home/user/pr629-evidence/ci-fast-postaudit.log`.
- The **first** fast-CI run was **not** green: sixteen unrelated x86-64
  MachInst tests selected the newly installed *i686* GAS 2.47 from a
  lexicographically sorted cache and fed it 64-bit registers. Fixed that
  test-only oracle discovery to select **only the x86-64 target** (or fall
  back to system GAS). The second run passed without an oracle override.

## Review AI's twelve assertions: accepted, narrowed or rejected

| # | Verdict and independent observation | Follow-up |
|---|---|---|
| 1. `{vex}` APX/early dispatch | **Confirmed.** GAS rejects `{vex} ccmpeq`, `ctestne`, `cfcmovbe`, `imulzu`; merged main emitted APX EVEX, e.g. `62 f4 84 04 39 c8` for `ccmpeq`. | Require an **emitted** C4/C5/8F XOP prefix after dispatch, covering early returns instead of making an incomplete second mnemonic table. |
| 2. i686 `{vex}` delegation | **Confirmed.** `{vex} vpdpbusd` must be `c4 e2 75 50 d3` in both modes; main's i686 code emitted default EVEX `62 f2 75 28 50 d3`. `{vex} vmovdqu8` must reject; plain `vmovdqu8` must remain valid EVEX. | Share the x86-64 hint initialization in i686 delegation, retain the mode-specific i686 address/prefix wrapper, and check the requested encoding there. Do **not** ban normal 32-bit EVEX/zmm. |
| 3. Nonvector `v*` instructions | **Confirmed, narrower than claimed.** `{vex} vmcall`/`vmmcall`/`vmlaunch`/`vmresume`/`vmxoff`/`vmfunc` were silently legacy-encoded. `{vex} vmread` already rejected, though with a different diagnostic. `{vex} verr` already rejected. | The emitted-prefix postcondition rejects the legacy cases; true XOP `vpcmov` and unprefixed-family `lwpins` **remain accepted** with `{vex}`. No broad `v*`-means-vector rule. |
| 4. EVEX B4/X4 mutation | **Design concern accepted, claimed byte failure not demonstrated.** Main matched GAS on EGPR-base/index and zmm31 controls. | Pass B4 and X4 into `emit_evex` structurally; remove the late four-byte-backpatch function. All existing and new EVEX cases remain byte-identical. No performance gain is asserted. |
| 5. `%ah` in broadcast | **Confirmed and expanded.** Main accepted `%ah` but also `%al`, `%ax`, `%rax` and invalid `%r8b` variants for `vpbroadcastb`; GAS rejects them. The valid **32-bit** source `%eax` already matched in x86-64, but main's i686 encoder emitted a **nonexistent VEX GPR broadcast row** rather than GAS's EVEX row. | For b/w/d require r32; for q require r64. Send the i686 broadcast family through the shared encoder. All 180 width/length matrix cases now agree with GAS. |
| 6. i686 `vmovddup` decorators | **Confirmed.** Main accepted `vmovddup (%eax){1to2},%xmm1` as EVEX although GAS rejects it, and similarly accepted invalid ymm/zmm broadcast decorators. | Run shared post-encode decorator validation once in i686's real 32-bit wrapper, including delegated vector instructions. Existing legal i686 memory forms still match GAS. |
| 7. Documentation | **Confirmed, corrected.** The i686 VEX module falsely claimed 32-bit mode has no zmm/EVEX support; an earlier session document falsely claimed i7-14700KF has AVX512-FP16. | Correct both statements and update stale differential-suite totals. AVX512-FP16 remains an assembler-coverage gap, not a runtime opportunity on the target 14700KF. |
| 8. `vmovddup` tuple/mixed widths | **Not a merged-main byte bug.** Main and GAS both encode `vmovddup -1024(%rdx),%xmm30` as `62 61 ff 08 12 72 80` (xmm tuple scale 8). Legal ymm/zmm controls also match. Both already reject `%ymm3,%xmm4`; diagnostic text differs. | Preserve the correct tuple encoder rather than copying a proposed rewrite; add explicit i686 decorator coverage under #6. |
| 9. `{vex2}` / `{vex3}` | **Confirmed with a key correction.** `{vex3} vaddps` is GAS C4 `c4 e1 68 58 d9` but main C5. However `{vex2} vpdpbusd` **legally uses C4**, since the 0F38 map cannot fit C5. | Keep distinct typed VEX selector variants through parsing, x86-64 emission, and local i686 GP-touching emission. VEX2 prefers C5 only when expressible; VEX3 forces C4. |
| 10. Incompatible hints | **Confirmed.** `{nf} {vex} vpaddd` and `{rex2} {vex} vpaddd` silently emitted plain VEX on x86-64; i686 also ignored APX-only hints. | Reject incompatible combinations before encoding, and reject `{nf}` / `{rex2}` in 32-bit mode. A rare `{rex2} {vex} movq` still rejects with a different GAS diagnostic. |
| 11. `enclv` | **Rejected for merged main.** Main and GAS both emit `0f 01 c0`; the previously alleged `0f 01 e0` encoding was fixed *before* this base. | Do not add an unnecessary opcode change. Keep existing control in the differential corpus. |
| 12. EVEX-only routing table | **Confirmed, concretely.** Literal-table inventory plus 48 actual GAS probes found **twelve implemented EVEX-only mnemonics unreachable** with low xmm/ymm registers: `vpabsq`, `vpandnd/q`, `vpconflictd/q`, `vpmaxsq/uq`, `vpminsq/uq`, `vpmullq`, `vpsraq`, `vpsravq`. The zmm spellings already reached the EVEX dispatcher. | Route these twelve names as EVEX-only; exact GAS bytes for the low-register cases are pinned on x86-64 and i686. Do not misclassify the dynamically matched FMA3 family, which has legal VEX rows. |

### Remaining diagnostic-only differences

Among the 88 focused two-assembler probes, **zero acceptance/byte
mismatches** remain, but **ten error-message strings differ**. These are
`{vex} vmread` (two modes, already rejected), `{rex2} {vex} movq`, six
`vmovddup` invalid-broadcast shapes (all rejected; GAS says *unsupported
broadcast*, LCCC *operand type mismatch*), and one mixed-width `vmovddup`
(the same operand-size rejection with extra register names). The exact
lines and both diagnostics are in
`/home/user/pr629-evidence/probes/final-probe-diagnostics.txt`. No claim
of complete x86 ISA coverage or universal diagnostic-text parity is made.

## Generated code and deliberately unclaimed performance

The only compiler-binary changes are assembler hint validation, legal
encoding selection, EVEX-only opcode reachability and tests/docs; there is
**no new C optimizer transformation**. A reproducible static check compiled
five unchanged P0/related C programs (`spectral_norm`, `mandelbrot`,
`linux_find_bit`, `lz4_compress` with match-rich inputs, and
`sha256_transform`) with both merged-main and candidate LCCC, using
`-O2` and `-O2 -march=x86-64-v3 -mtune=raptorlake` on the same VM.
All **10/10** resulting object `.text` sections and relocation listings
were byte-identical: see
`/home/user/pr629-evidence/performance-text/compare_objects.py` and
`comparison.json`. Identical machine code gives **no basis for a runtime
speedup claim** on those programs; a timing run of identical binaries would
be host noise. The available Xeon VM is not the required i7-14700KF, and
the prior GCC/Clang 23.1/ICC/ICX Godbolt assembly comparisons in
`engineering/evidence/2026-09-26-followups/README.md` cannot substitute
for new paired target-hardware measurements. The byte-compare,
Mandelbrot, `linux_find_bit` and allocator P0 items therefore **remain
open** in `backlog.md`. A speculative widened LZ4 read or allocator rewrite
would violate the range-proof and no-regression requirements; none is
smuggled into an assembler correctness patch.

**Reproduce the local validation** (with an active swap file and adequate
Rust RAM headroom):

```sh
export CARGO_HOME=/home/user/.cache/lccc-cargo
export RUSTUP_HOME=/home/user/.cache/lccc-rustup
export PATH="$CARGO_HOME/bin:$PATH"
export CARGO_PROFILE_FASTBUILD_DEBUG=0 CARGO_INCREMENTAL=0 CI_LOCAL_JOBS=1
scripts/ci_local.sh --fast
python3 scripts/asmdiff.py --jobs 2 \
  --as /home/user/.cache/gas-2.47-x86_64-linux-gnu/bin/as \
  --lccc target/fastbuild/lccc-x86
python3 scripts/asmdiff.py --32 --jobs 2 \
  --as /home/user/.cache/gas-2.47-i686-linux-gnu/bin/as \
  --lccc target/fastbuild/lccc-i686
```

The GAS 2.47 binaries can be installed with `scripts/ensure_gas_247.sh` if
not already cached. Machine-wide package setup (multilib, if needed for
32-bit *linking*) is independent of the assembler byte comparison.
