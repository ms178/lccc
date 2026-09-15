# FOLLOWUP — PR #536 ICF adoption + oracle rebuild (2026-09-15, icf-adopt branch)

## What landed

Branch `icf-adopt` on top of `d40b2593` ("Merge pull request #538"):

| Commit | Content |
|---|---|
| `3d3279d7` | PR #536 core: iterative class-based ICF for lccc-ld (safe/all modes, opcode-aware address-taken, 42 unit tests, 100K-fn scale corpus) |
| `3b49f2bf` | PR #536 linker suite + docs (+245 tests) |
| `68d58123` | Oracle pinning: tools/linker/setup_oracles.sh + artifacts restore path |
| `a99ae2c3` | **Fix A**: opcode-gate for short branches (0xeb/0x70-7f): upstream docs claimed short-jumps are control flow; the gate only covered 0xe8/0xe9 far forms |
| `496b745e` | **Fix B + worklist**: named-global relocation targets compare by the class of their chosen definition (monotone-refinement sound); fixpoint convergence switched from capped full passes to a dirty-class worklist |
| `83182a94` | rustfmt pass |
| `ab609812` | clippy: upstream PR test code `cloned_ref_to_slice_refs` (PR never ran clippy — no CI) |
| `bff0a2f4` | regression corpus: **latent OOB write in `cmp_replay_acc_nohome` fixture** (upstream commit `a041f347`) — `ring[64]` held 80 bytes of live objects; #538's layout change unmasked it at -O2 (SEGV @ 0x405000, bss end). NOT a miscompile: gcc got "lucky" placement, lccc didn't. Fixture fixed to `ring[80]` with comment |
| `8319cdd6` | linker suite: un-skipped `reloc_64_accepts_above_4g` (bare `--defsym` is a linker spelling even GNU gcc rejects at driver level → `-Wl,--defsym,…`; the printed value is `%llx` hex, expectation was decimal) |

## Validation status (all green, 2026-09-15 ~14:30Z)

- `cargo test --lib`: **2861 passed**, 0 failed, 6 ignored
- icf tests: **42/42** (incl. 2 new: `deep_twin_chains_fold_fully`, `deep_twin_chains_bounded_under_exhaustion`)
- linker suite (`run_linker_tests.py`): **239 pass / 0 fail / 0 skip** (was 238/0/1 with the historical skip)
- regression corpus (`CCC_VALIDATE_SSA=1`): **762 pass** (fixture fix) / 9 skipped_compare
- `ci_local.sh` full (incl. clippy + two slow oracles): **39 passed / 0 failed / 0 skipped**
- 100K-fn scale corpus race vs oracles (all linkers on identical archive):

  | Linker | Mode | ICF bytes saved |
  |---|---|---|
  | wild/bfd | (no ICF support) | — (`.text` 538,082 baseline) |
  | mold 2.42.1 | all | 642,375 (`.text` 483,554 saved vs baseline) |
  | **lccc-ld** | all | **642,409** (`.text` 446,509; folds ≥ mold, +34 bytes) |
  | **lccc-ld** | safe | 642,407, `rejected_unsafe=1` (the address-taken sentinel — correct) |

- Worklist effect: comparisons 2,138,494 → **159,088** at byte-identical output (sha-verified in-session)
- safe/all invariance of `.text` sizing verified; the two images differ only by the one legitimately rejected fold

## Environment lessons (wipe playbook — update if drift)

Wipes reset everything outside /home/user INCLUDING apt packages and /tmp:

1. `bash /home/user/setup-swap.sh` (swap is mandatory before builds; check /proc/swaps)
2. `sudo apt-get install -y -qq gcc-multilib g++-multilib libc6-dev-i386 gdb cmake ninja-build`
   (the five i686/multilib CI gates fail at `bits/libc-header-start.h` without the multilib set — masquerades as codegen regressions!)
3. Artifacts-restore path: rust in `artifacts/rust` (cargo registry cache may be partially wiped → `rm -rf $CARGO_HOME/registry/{src,cache}` forces clean re-fetch)
4. `bash tools/linker/setup_oracles.sh` restores oracles from artifacts (fast) or rebuilds (~9 min total on 2 cores from source caches)
5. 100K corpus regen: `python3 scripts/icf_scale_corpus.py gen /tmp/icfwork` (~60 s)
6. Rebuild rust binaries: `bash scripts/build_lccc_fast.sh` after `source /home/user/artifacts/bin/env.sh`

## Open items

- Oracles' binary sha256 in ORACLES.md refreshed after rebuild (same source pins: bfd 2.47.20260726, mold 2.42.1, wild c13068b)
- `#538` (arena/01a0a48d) touched regalloc/SLP; it broke **nothing** in the gates once env was restored (the initial phi/gla/tls "regressions" were the multilib wipe). Keep an eye on it: it is a big codegen patch merged without the oracle races this branch went through.
- `ms178-1.patch` ultimate patch: regenerate via scripts/lccc-snapshot.sh (S11) after final seal.
- wild does not implement `--icf=all` (warns + ignores) and not `--icf=safe`; bfd has no --icf at all. For ICF equivalence work mold remains the only meaningful oracle.
