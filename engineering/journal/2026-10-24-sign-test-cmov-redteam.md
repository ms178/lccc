# 2026-10-24 — red-team hardening of the sign-test fold

Base: `4ba567022b7d358837aa13be4eb999621871c9f9` on `main`, unchanged at the
start and end of this pass. The previous receipt added a narrow x86-64 peephole
for `arithmetic result; test R,R; cmovge/cmovl`. This pass re-audits that work
with the build enabled, closes every defect the audit found, extends the algebra
to the 32-bit branch lowering, and makes the differential gate directly wired,
architecture-aware and reproducible.

## What the red team found

1. **The kill switch did not switch anything.** The pass was registered under the
   existing `self_test` skip group, while the regression helper set
   `CCC_PEEPHOLE_SKIP=fold_arithmetic_test_sign_cmov`. `sk()` matches exact
   names, so the A/B compile was identical to the optimized compile. The helper
   still passed because its assertion counted a `testl` anywhere in the
   translation unit — the strict-positive negative control in the same fixture
   legitimately keeps one. Measured: both disable and optimized assemblies had
   `cmovnsl`/`cmovsl` and one surviving `testl`; the "disabled" compile proved
   nothing.
2. **The later-consumer licence was stronger than the proof and weaker than the
   comment.** It passed `preserved = ZF|SF|PF` to the CFG walk but then vetoed
   *any* later consumer, including a benign `sete`. It also only accepted
   `ge`/`l`, refusing `g`/`le` even for a flag-for-flag exact logical producer
   whose CF and OF already equal TEST's zero. `andn` was conservatively
   classified as merely ZF-agreeing despite clearing CF/OF.
3. **The gate was an allowlisted orphan.** `scripts/ci_gate_allowlist.txt` is
   documented as a shrinking debt record, yet the new helper was parked there
   and reached only through a wrapper option. That hid the helper from the
   parity checker's path layer and made the wrapper a second source of truth.
4. **Local gate tool names were hard-coded** to `gcc-16` and `clang` in one
   helper path while the rest of `ci_local.sh` resolved `GCC16`/`CLANG`. ICX
   was also evidence-local/optional with an absolute sandbox path recorded in
   the manifest, while a real authenticated ICX 2026.2 job already existed in
   hosted CI.
5. **The optimization was x86-64-only.** The i686 backend lowers the same
   source to `sub; test; jl/jge` branches and carried the identical redundant
   TEST. No 32-bit fold existed, and no differential test exercised `-m32`.
6. **Parity tests lacked an orphan-gate mutation.** `check_orphaned_gates` had
   no direct negative test; deleting the guard could leave the real tree green.
7. **A stray documentation line** describing the induction-bump helper had been
   absorbed into the preceding fold's doc comment, and the lane-plan cost
   metadata was stale for the newly wired gate.

## Fixes, with their proof boundary

- **Dedicated switch and honest A/B.** The x86-64 pass now owns
  `CCC_PEEPHOLE_SKIP=sign_test_cmov`; the general `self_test` group no longer
  hides it. A unit test pins that `sign_test_cmov` restores the TEST while
  `self_test` does not. The regression helper asserts per-function bodies and
  compares the skip-group TEST count against the optimized count, so a dead
  pass cannot hide behind the strict-positive control.
- **Exact-flag producers and a refined CFG licence.** `and`/`or`/`xor`/BMI1
  `andn` are now `TestFlagAgreement::Exact`; arithmetic producers remain
  `ZeroOnly`. `cmovge`/`cmovl` (and synonyms `nl`/`nge`) map to `ns`/`s` for
  both classes; exact producers additionally admit `cmovg`/`cmovle` unchanged.
  A new `ConsumerFacts::saw_outside_zf_sf_pf` fact lets a later `sete`, `js`
  or `jp` survive the arithmetic fold while `setl`, `adc`, `pushf`, inline asm
  and unresolvable edges still veto. Whole-flag readers always veto arithmetic
  folds.
- **Direct, mirrored gate.** `check_sign_test_cmov.sh` is invoked directly by
  `ci_local.sh` and by the hosted `backend-contracts-b` job; the allowlist entry
  is gone and the wrapper's `--sign-cmov-only` option is removed. The parity
  checker reports 373 commands and 27 contracts, all passing. Four mutation
  tests now pin orphan discovery, allowlist staleness and both wiring directions.
- **Resolved oracle/tool overrides.** The gate accepts `GCC_SIGN_CMOV`,
  `CLANG_SIGN_CMOV`, `ICX_SIGN_CMOV`/`LCCC_ICX_BIN`, `LCCC_FASTBUILD_BIN` and
  `LCCC_SIGN_CMOV_HARNESS_DIR`. The local mirror resolves the same `GCC16`/
  `CLANG` variables as the rest of the fast gate. ICX is verified by the
  `__INTEL_LLVM_COMPILER` macro when present, and the hosted `icx-oracle` job
  now compiles and runs this exact fixture with its authenticated snapshot and
  then runs the full differential gate with `ICX_SIGN_CMOV` exported. Local
  sandbox provisioning of ICX 2026.2 remains blocked by the egress allowlist
  (TLS EOF to Intel's APT host and to GitHub release-asset hosts); this is
  reported as a skip, not silently treated as coverage.
- **i686 twin.** `fold_arithmetic_test_sign_jcc_i686` rewrites `jl`/`jge`
  (and synonyms) to `js`/`jns` after an equal-width self-TEST, admits exact
  logical producers for `jg`/`jle`, and runs a bounded CFG walk from both
  successors. The walk ends each path at a complete flag writer, call or return;
  a later conditional jump, SETCC/CMOV, partial writer, indirect jump, inline
  asm or unknown mnemonic refuses. It has its own `CCC_NO_I686_SIGN_JCC` kill
  switch and nine unit tests.
- **Documentation, metadata and evidence hygiene.** The induction-bump doc
  comment is repaired. The lane plan gains the measured 2 s gate cost and
  updates the backend-contracts total. The evidence manifest keeps its source
  digest and pinned ICX package digests but no longer records an absolute local
  binary path; the hosted artifact name and probe replace it. A dedicated
  journal entry records this audit.

## Tool-based verification (all commands run with the 8 GiB swap active)

| Check | Result |
| --- | --- |
| `scripts/build_lccc_fast.sh` (Rust 1.97.0, clang driver + mold, `-D warnings`) | clean, 3m20s incremental |
| `cargo test --profile fastbuild --locked -- --test-threads=2` | 6156 passed, 0 failed, 10 ignored (6166 lib tests + binaries + doctests) |
| Targeted sign-CMOV unit tests | 17 passed |
| Targeted i686 sign-JCC unit tests | 9 passed |
| `tests/regression/check_sign_test_cmov.sh` | PASS, 2 s: 5 opt levels x GCC 16.2.0 / Clang 23.1.3 runtime, codegen, skip group, i686 codegen + runtime |
| `python3 scripts/check_ci_gate_parity.py` | PASS (373 commands, 27 contracts) |
| `python3 scripts/test_ci_gate_parity.py` | 82 passed (4 new orphan mutations) |
| `python3 scripts/test_ci_lane_split.py` | PASS |
| `bash -n` on all changed shell scripts | PASS |
| `python3 -m json.tool` on lane plan and manifest | PASS |

No runtime-speed claim is made for the peephole itself: it removes one static
instruction per matched site. The gate's 2 s measurement is CI cost, not a
benchmark of compiled code. Broader scalar min/max/abs lowering and PMU-level
confirmation remain outside this receipt.

## Residual risk, explicitly stated

- Local ICX 2026.2 cannot be provisioned in this sandbox; hosted CI is the
  authoritative ICX leg and is wired above. A local researcher can opt into
  `LCCC_SIGN_CMOV_PROVISION_ICX=1` when the Intel repository is reachable.
- The i686 CFG proof is deliberately conservative: it vetoes any later
  conditional jump even when that jump would only read ZF. This costs a few
  theoretical folds and keeps the proof auditable; the same refinement applied
  to x86-64 is a possible future, separately measured, follow-up.
- The optimization remains x86/x86-64 only. ARM and RISC-V do not use EFLAGS
  condition codes and are out of scope for this transformation.

## Restored-workspace follow-up

After the workspace restore, the tool caches and test logs were absent. The
results above describe the earlier run, not a fresh run in the restored host.
The gate-parity and lane-split suites were rerun after reinstalling PyYAML.
A fresh inspection found two additional proof defects, now corrected:

- ANDN leaves PF undefined, unlike TEST. It is no longer classified as a
  TEST-equivalent producer on either backend. Earlier ANDN/exact claims above
  are superseded by this correction; logical AND/OR/XOR remain eligible.
- The i686 CFG walk must not terminate at shifts: zero or masked-zero counts
  preserve incoming flags. Nor may it walk past PUSHF/PUSHFL as neutral stack
  instructions. Shifts now fail closed, and stack-neutral matching is exact.
  A regression matrix covers immediate-zero, masked-zero and variable shifts,
  plus both flag-snapshot spellings followed by a carry reader.
