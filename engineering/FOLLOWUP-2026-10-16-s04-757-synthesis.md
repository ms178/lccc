# S04 follow-up: closed-PR audit, #757 re-land, and the two main bugs it exposed

Session branch: `arena/d3169e6d-lccc`, PR #844. Base: main `e8ae10ab` (merge
of PR #843). Companion to `FOLLOWUP-2026-10-16-rebase-synthesis.md` (S03).

## What landed (commit order)

| commit | content |
|---|---|
| `71ea23ee` | A13/A14 libcall policy (from #757 P1), 5 fixtures |
| `774da9e3` | Sparse NOBITS `.bss` (from #757 P2), measured RSS table in the message |
| `a185ee1b` | RA-01 sym+reg SIB fold (from #757 P3), seg exclusion is NEW |
| `324f6fa7` | reloc 11 = `R_X86_64_32S`, not `@GOT64` — -fno-pie jump tables SIGSEGVed |
| `5305a28d` | RA-01 gate + fixtures; dormant-fold RA-neutrality (static deadening preconditions in the link extension) |
| `804b73f4` | Array-bound contract (from #757 P4): decay, u128 byte-size, -m32 truncation |
| `020b8a59` | Gates local+hosted (parity), sparse-bss gate case 6 re-measured against GAS 2.47 |
| `07c64842` | `__CHAR_UNSIGNED__` derives from `char_is_unsigned()`; char gate 11/11 |

## Decisions that differ from #757 (with the measurement)

1. **Sparse gate case 6**: 757 demanded a diagnostic for `.p2align 40` at
   offset 0. GAS 2.47 ACCEPTS it (752-byte object, `sh_addralign 2^40`,
   measured) and lccc agrees byte-for-byte; only the padding-bearing variant
   (`.byte 1` first → 2^40-1 fill bytes) is diagnosed, by GAS and by lccc.
   The gate now pins both measured subcases. Rule of the whole session:
   757's comments were written against binutils 2.40/2.44 — main's GAS-2.47
   remeasurements won every conflict.
2. **RA-01 link extension**: 757 linked `ro.off` on map membership alone
   ("harmless over-constraint"). Against main's drift that moved the RA of a
   DORMANT build (+1 copy in ra01_global_match). The extension now replicates
   the deadening pass's two static preconditions (every use of the `Add` is a
   mapped indexed-GEP base; no store consumer) — RA-independent, so an `Add`
   failing them can never need the offset register at the access.
3. **RA-01 seg exclusion** (not in 757 at all): main's seg-indexed emit
   ladder postdates the PR and rebuilds refused addresses via
   `rematerialize_skipped_indexed` from the GEP BASE home — a home a deadened
   `Add` never writes. `build_reg_offset_sym_map` drops any `Add` reachable
   from a seg-qualified access (directly or as GEP base).
4. **Option contract NOT ported** (deliberate): `LCCC_STRICT_OPTIONS`, the
   fail-closed data-model/dialect rows, the `-D_FORTIFY_SOURCE` diagnostic
   and PR 757's capability-probe script (`lccc_capability_probe.py`, never
   landed on main) belong to 757's cli.rs hygiene part
   (deferred). `docs/builtin-libcall-policy.md` §"Option contract" documents
   the CURRENT behaviour and names the gap. Measured today:
   `-funsigned-char`/`-std=<unknown>`/`-fno-plt`/`-D_FORTIFY_SOURCE=2` are
   accepted, `-fstack-protector` exits 1.
5. **757's script bugs fixed before registration**: undefined name `bss` in
   a reject-path f-string of `check_array_bound_contract.py` (masked only by
   loop-variable leakage), unused import/locals — pyflakes clean now.

## Main bugs found while validating (both fixed here)

- **reloc-11 GOT misclassification** (`e3e7434d` regression): every
  `-fno-pie` switch jump table read from absolute address 0. Found because
  the corpus A/B for RA-01 ran under `-fno-pie`; four programs SIGSEGVed
  with the MAIN binary too (worktree-verified before blaming the port).
- **char-signedness three-way drift**: type said unsigned, macro said
  signed, limits.h followed the macro.

## Closed-PR verdicts beyond #757

- **#714 (loop IV un-spill + per-path flag walk): NOTHING TO PORT.**
  Part 2 (per-path LIVE-mask flag walk) is ABSORBED on main - commit
  `20b212e0` carries the same design and even the same `cmp; jb; ...;
  stc; jo` example in its docstring. Part 1 (`loop_iv_spill`, 1883
  lines) was ported experimentally and REVERTED: main's RA drift
  already keeps the hot match-loop IV in a register (no small-loop
  slot round-trip left in lz4_compress; measured, the pass fires 0x in
  the corpus and its own e2e gate fails as "vacuous"). The remaining
  lz4 gap vs gcc (1209 vs 1067 ms, 0.88x, MATCH_RICH/PASSES=2048,
  identical checksums) is NOT the IV round trip - root cause still to
  be found (open lead below). The outer-loop `184(%rsp)` round trips
  that do remain are one store/reload per OUTER iteration and do not
  fit the pass's header-opens-with-reload model.

## Still open (carried forward)

- cli.rs option-contract port + capability probe (see §4 above).
- lz4_compress 0.88x vs gcc-16: profile the match kernel (not IV spill).
- Deep-dive queue by novelty: #712, #816/817, #766/767, #709/705/706,
  #777, #833, #835, #779/778, #814, #697, #831, #828, AArch64 #761.
- Harness: Clang-23.1 leg, godbolt-oracle script, i686-mold preset,
  `.flags` -O2 audit, ICX-2026 PPA, latest-rust attempt.
- Weak kernels vs gcc (geomean 0.661x): classify 0.27x, namechars 0.25x,
  seg_gs_fill 0.36x, seg_gs_copy 0.42x, seg_gs_sum 0.51x, varint 0.66x,
  strcmp_signed 0.69x.
- i686 GOT32 → GOT32X relaxable spelling (documented ledger item in the
  got-slot gate).

## Session traps worth re-reading

- `git apply --3way | head` dies on SIGPIPE and reverts ATOMICALLY — the
  "Applied cleanly" lines lie. Run it unpiped, inspect after.
- Benchmark corpus A/B needs a timing-text exemption (i686_alu_chains
  prints seconds).
- Commit messages with backticks must go through `-F file`, never `-m` with
  a double-quoted heredoc-less string (shell substitution ate `ro.off`).

## CI overhaul (2026-10-16, cost-driven)

Measured on run 38967786415: the ubuntu-toolchain-r PPA download dominated
wall time — "Install GCC 16 and i686" cost 15.2 min in build, 15.0 min in
debug, ~4 min elsewhere (~37 runner-minutes per run), while the actual
build (2.1 min) and tests were small. Changes, each with its contract
covered by an existing pin test:

- `scripts/ci_install_gcc16.sh` honors `LCCC_CI_APT_ARCHIVE_DIR`: apt keeps
  its downloaded debs there (`Dir::Cache::archives` +
  `APT::Keep-Downloaded-Packages`), the workflow persists the directory in
  `actions/cache` under a weekly key with a prefix restore. apt still reads
  the live index first, so a stale cache costs only the delta download and
  can never pin an outdated toolchain (rolling-major policy unchanged).
  Mock coverage: `tests/regression/check_ci_gcc16_provisioning.py`
  (archive-dir test asserts the option set and that `apt-get update` stays
  unredirected).
- Removed true duplicates from `ci.yml`: the RISC-V shared-binding pair ran
  three times (frontend named steps, codegen pr835 block, backend step) and
  `scripts/check_strict_packed_recip.py` twice with identical effective
  configuration (CCC default equals the backend env). One copy each
  remains; `scripts/test_ci_gate_parity.py` stays green.
- The label-gated "Named candidate semantic and performance audit" job
  moved from `pr826-redteam.yml` (which triggers on every PR push and thus
  showed a permanent skipped check) into `compiler-comparison.yml`,
  triggered only by the `compiler-comparison` label event or
  `workflow_dispatch`. Opt-in semantics and token scope are unchanged and
  still pinned by `tests/corpus/test_compiler_comparison_reports.py`.
  Note: after labeling, later pushes do not re-run the audit automatically;
  re-apply the label or dispatch.

Expected effect: build lane ~20.5 → ~6 min, debug ~22 → ~8 min, critical
path (build + codegen) ~39 → ~20 min once the weekly cache is warm; first
run of a week still pays one download to refill the cache.

## Census note: RA-01 kill switch

`CCC_NO_REG_OFF_SYM` disables the RA-01 sym+register SIB fold in
`src/backend/generation.rs` (set it to compare against the unfused
addressing form). Added with the RA-01 port; documented here so the
audit census counts it as a named, owned knob.

## Latent main divergence surfaced by the local suite (not by hosted CI)

`fenv_constant_effects` / `fenv_constant_rounding` (in main since #843)
fail the GCC runtime comparison under `run_regression_suite.sh` — and
assembly-level analysis shows the mismatch is GCC's, not ours: LCCC keeps
the runtime `vaddsd` for `0x1p0 + 0x1p-53` under `FENV_ACCESS ON`
(conformant; its self-check exits 0 in every mode), while GCC 16
constant-folds to the round-to-nearest value and its own self-check exits
1 under FE_UPWARD; under `-frounding-math` GCC also DCEs the discarded
`1.0/0.0` (exit 52) where LCCC preserves the FE_DIVBYZERO raise. The
differential is therefore wrong-by-design for these two programs:
both now carry `LCCC_NO_COMPARE=1` sidecars, and the exit-code-enforcing
conformance coverage (every -O level, pragma and flag routes, both ABIs)
lives in `tests/regression/check_fp_environment.sh` in both CI mirrors.
Hosted `run_regression.py` had always recorded SKIP-COMPARE here (its gcc
invocation fails to link `-lm`), which is why the divergence only
surfaced through the local suite.
