# Follow-up work — 2026-09-30 (integration session: #678 adjudication + CMP-REPLAY onto the #681/#682 union)

Deliverable: single commit `09842649` on top of `58282379` (the union of PR
#681 and PR #682 = the "merged upstream" work). 74 files, +1720/−347.
Review adjudication: `engineering/PR678-REVIEW-2026-09-30.md`.

## Done and verified in this session

1. **AUD-1 volatile-licm miscompile** — fixed (guard restored before the
   dominance check), gate `check_volatile_licm.sh` in ci.yml + ci_local.sh
   (parity 97), red-on-pre-fix / green-on-fixed verified both ways.
2. **AUD-2 false "AArch64-only" narrative** — corrected in all 5 documents;
   argument-identity of the two late entries stated plainly.
3. **AUD-3 dominance hole in div/LShr preheader hoist** — strict-CFG
   precondition at both AVX2 and SSE2 sites; diagnostics reuse the
   function-scoped `debug` binding (no new env reads).
4. **AUD-4/6 twin late entry** — routed through the same scoped entry:
   single analysis run, DynAlloca/volatile bails inherited, flag scoped.
5. **AUD-7 orphaned fn deleted; AUD-8 RAII scope guard.**
6. **AUD-5** kept + disclosed (measurement debt → follow-up 1 below).
7. **CMP-REPLAY peephole** (`redundant_cmp.rs`, wired after setcc_cmov,
   kill switch `CCC_PEEPHOLE_SKIP=redundant_cmp`, 12 unit tests): corpus
   A/B −8 static insns over 51 programs, zero growth; varint shape 16→15
   insns / 2→1 cmp with bit-identical output.
8. **Lint residuals** (unused `inst`/`pat`) fixed; rustfmt clean; clippy
   clean on the final tree (the mid-run failures were in-flight-edit
   artifacts, re-run green).
9. **Env-read ratchet** 157→158 with the kill switch named as the one
   sanctioned step; the two AUD-3 diagnostics cost zero reads.

## Validation status at handoff

- Unit tests 3890/0; minmax-shapes 14/14 at exact densities; all volatile
  + loop-preheader + minmax-reduction gates green; corpus green except the
  six i686-multilib environmental failures (no i686 libc, no root on this
  box).
- Full `ci_local.sh` re-run on the final tree: **in flight at handoff**
  Expected: green except the environmental
  classes below.
- Oracle hard data: see PR678-REVIEW-2026-09-30.md §6.

## Follow-ups (in priority order)

1. **AArch64 `-ffast-math` late-entry A/B** (AUD-5 debt): needs a runner
   with the aarch64 cross toolchain; then either keep-with-measurement or
   pin the old dispatch arm. Owner: next session with cross toolchain.
2. **Runtime A/B of CMP-REPLAY on the branch-shape kernels**
   (varint 0.64x, memchr 0.53x, namechars 0.56x vs clang): static count
   dropped 8/7645 corpus-wide; the runtime question is whether the
   replay-removal moves those geomean factors. Use `scripts/bench_kernels.py
   --baseline <union-baselines.json> --save cmp-replay.json` with
   `--lccc` pointed at the pre-wired vs wired binaries.
3. **Full green stamp on a properly provisioned box**: i686 multilib (6
   corpus tests) + GAS 2.47 oracle (asm-diff gates; the binutils build tree
   at ~/.cache/binutils-2.47-build-* exists and was being finished on this
   box — resume `make && make install` there before re-running
   x86-asm-diff/i686-asm-diff).
4. **Density outliers**: nbody (188), moving_stats (90), i686_alu_chains
   (86) gap to best oracle — next oracle hard-data pass.
5. **Static rank re-measure after CMP-REPLAY** across the 78-function set
   to confirm the 60-behind count improved (binary_search should move).
6. **Oracle cache hygiene**: `scripts/godbolt_cache.py` namespaces are
   warm (147 entries in the old tree's .godbolt-cache/); refresh on the
   merged tree when the godbolt oracle gate runs end-to-end.

## Not done (deliberately)

- No changes to the mode wrapper's pre-conversion (AUD-2 code was correct;
  only the narrative was fixed).
- No runtime measurement of the AUD-5 AArch64 arm (no cross toolchain).
- No further peephole passes in this commit beyond CMP-REPLAY (P2 scope
  discipline, disclosed).
