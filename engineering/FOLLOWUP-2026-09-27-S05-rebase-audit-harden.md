# Follow-up: S05 rebase onto e265e05 + full line-by-line audit + hardening

**Base:** upstream `e265e05` (PR #645 merge; session started from the S04
tree at `a864627` + uncommitted S01–S04 content + S04 adoptions). This
session recovered from a turn-boundary wipe (no `.git`, no toolchain
shims, no build, dead background CI), rebased onto latest main, audited
every line of the rebased delta (26 files), fixed all defects found,
proved the fixes with targeted + mutation tests, re-ran the full CI
mirror, and re-benchmarked on the quiet box. Patch: `ms178-1.patch`
(snapshot entry `S05-…`, base `e265e05`).

## S05.0: environment recovery (all pre-existing harness gaps, fixed)

- Swap: recreated 8G `/swapfile` via `scripts/ensure_swap.sh` (was 0).
- Toolchain: `rustup` shim had lost its link siblings (snapshot keeps one
  hardlink per inode: only `rustup` survived, cargo/rustc/rustfmt/clippy
  MISSING despite an installed `stable-1.98.1` toolchain). Repaired with
  `ln -f rustup <shim>` × 7; all dispatch verified. Permanent fix in
  `arena_session_restore.sh` (F2a): recreate missing shims after toolchain
  install, FATAL if cargo still unusable. Validated by simulation on a
  scratch `CARGO_HOME`.
- `.git` recovery: restore fell back to an upstream `--depth 200` clone
  (`HEAD=e265e05`, mixed reset, worktree untouched, 0 deletions) because
  cloning `artifacts/lccc.bundle` fails. Root cause (proven by verbose
  re-clone): the S03 bundle was cut from a shallow clone, so it names
  prerequisites the receiver lacks — `git bundle verify` still says
  "okay", but the clone dies with "remote did not send all necessary
  objects". Two sessions of silent fallback. Permanent fixes:
  - `lccc-snapshot.sh` (F3): `fetch --unshallow` under `timeout 180`
    before bundling (silent best-effort), then a real test-clone of the
    published bundle; failures flag `bundle_clone=FAIL` in the ledger and
    warn loudly instead of blocking wipe protection. This session's
    bundle verifies `bundle_clone=OK`.
  - `arena_session_restore.sh` (F2b): bundle/upstream clone errors are
    captured and the `error:`/`fatal:` line is logged (proven against the
    broken S03 bundle: `error: Could not read da964ed…`, not hint noise);
    `-c init.defaultBranch=main` on both clones.
  - `lccc-snapshot.sh` (F14): atomic publishers `chmod 644` (mktemp's
    0600 leaked onto the deliverable and ledger).
- S04's background full-CI died with its sandbox at the cargo-test gate;
  S05 re-ran the mirror from scratch (see CI verdict pointer below).

## S05.1: rebase onto latest main (`e265e05`)

Upstream moved by exactly one PR since `a864627`: #645 (`1d3349b`,
merged as `e265e05`). #642 and #626 remain unmerged. Rebase method: fresh
shallow clone → `git archive e265e05` → `diff -rq` vs worktree, cross-
checked against `git status` after restore (mode-noise subtracted:
195 − 170 = 25, exact match).

- **#645 adoption evaporates (best outcome):** all 10 adopted code/test/
  script files are byte-identical to the upstream merge (sha256 10/10).
  The 4 remaining #645 files differ only by S05-carried hunks (M1 in the
  three parity scripts; nothing in DECISIONS after F5).
- **F5 (rebase debris, required):** the tree deleted upstream's 69-line
  `PERF-REASSOC-LAG / CI-MIRROR` DECISIONS entry (S04 had skipped the
  file at adoption time). Restored verbatim from `e265e05`; the tree
  carries no unique DECISIONS content. Its gate-ruler numbers (55/56)
  are internally consistent (see §S05.2) and were left untouched.
- `.base_ref` mystery closed: `lccc/.base_ref` (`8ca2fd4` = PR #544
  merge) is an upstream-tracked file, not a session marker; the session
  marker is `artifacts/.base_ref`, re-anchored `e9f572e` → `e265e05` by
  this snapshot (`LCCC_BASE_REF`, merge-base guard satisfied).
- Rebased patch: 26 files (18 modified + 8 new), +2243/−249, all hunks
  audited below. `ms178-1.patch` is `APPLIES-CLEAN` against `e265e05`
  (snapshot-time apply-check in a verification worktree).

## S05.2: census-ruler correction (the col-0 branches)

lccc's x86-64 emitter prints some instructions (observed: conditional
branches) at column 0. Every S04 static count used a `^\s+[a-z]` ruler
(the gate counter and the H2H census shared it), which is blind to them.
The strip-based `codegen_oracle.py::_stats` recounts:

- rot x86-64: 55/55/54/55 → **57/57/56/57** (+2 uniform; adopted = 57).
- sha256 x86-64: 144/144/143/144 → **152/152/151/152** (+8 uniform).
- i686 counts unchanged (that emitter indents everything).
- S04 load counts withdrawn (the old load regex measured something else;
  true adopted-rot loads are 18, not 2). No decision rested on loads.

Oracle sides (GCC/Clang/ICX indent everything) re-verified unchanged:
pristine-13.2 55, hosted-13.2 56, 15.3 75, 16.2/trunk 60, Clang 23.1 85,
ICX 136. Corrected margins (lccc 57): −2 vs pristine 13.2 (was "tie"),
+3 vs 16.2 (was +5), +18 vs 15.3 (was +20; 26.04 migration still SAFE).

Why every S04 decision stands (full analysis in the S04 doc's S05
addendum): gate comparisons are same-ruler both arms; the adoption rested
on wall clock (n=21 interleaved, ruler-independent) and latency bounds
(`loop_latency.py` strips lines — re-audited; its `pick_loop`/`analyse`
handle col-0 correctly). New strengthening datum: GCC 13.2's own inner
rot loop bounds at **5.00** (`--label .L5`), so lccc's 2.00 loop is 2.5×
better-scheduled than the compiler winning the static count. The +2
static gap (prologue spills + loop shape, not schedule) is follow-up
work, optional because runtime already wins.

Deliberately NOT done: "fixing" the gate ruler or the emitter's
indentation without closing the real gap turns CI red with no codegen
change; weakening the check would be gate-gaming. The gate keeps working
as a regression tripwire (it caught S24's lost `addl (mem)` fold, which
is visible under both rulers).

## S05.3: line-by-line audit verdicts (rebased delta, all 26 files)

Verdict scale: KEEP (sound as-is) / KEEP+FIX (sound core, fix noted).
Every claim below was checked against code, tests, or oracle runs.

- `ci.yml` (M1 + #642 upload-to-end): KEEP. Step rename + casefile drop
  correct; upload-to-end publishes the closing dbgassert gate's artifacts
  (`if: always()`), no clobber (disjoint filenames). F1 fixed the one
  stale comment ("isolated PR #629 … 90-case" → whole-corpus wording);
  YAML parses, parity PASS (77 cmds), unit 15/15.
- `check_ci_gate_parity.py`, `ci_local.sh`, `test_ci_gate_parity.py`
  (M1): KEEP. Enforcement is exact set-equality on `.casefile` operands,
  so the empty spec genuinely requires whole-corpus; the narrowing
  mutation (append a casefile) is correctly rejected. `ci_hosted_only.txt`
  holds only the measurement-only bench script.
- `common.rs` (rax-analyzer stream diagnostics): KEEP. Pure classifier
  preserves every arm's verdict; stream-scoped records replace hidden
  thread-local state; the read-first test directly covers the old hazard.
  Dead-code audit: crate-level `allow(dead_code)` is upstream policy, so
  the tests-only wrapper is warning-free; release footprint is nil.
- `elf_writer_common.rs` (dot/PC8/diff): KEEP + F7/F7b. `dot_anchor` is
  map-idempotent; synthetic-name collision class equals the module's
  pre-existing duplicate-label behavior (documented, no action). The
  `Result` conversions fix two real misassembles (release truncation of
  out-of-range short jumps; disp-0 fallthrough jumping into the next
  instruction). PC8 range/diff arithmetic re-derived by hand against the
  unit tests (RELA addend −1: fwd 300, bwd −303 = …fed1 ✓). All three
  survivor paths gate on `unrepresentable_quad_error` +
  `externalize_pc8_reloc`. Nested-None (INTERNAL without real PC8) is
  unreachable on both archs (x86-64: Some/Some; i686: None/Some) — F7
  pins the invariant with a `debug_assert`, F7b executes it under the
  assertions-on harness (proven live by panic-probe; see §S05.4).
- `i686/encoder/mod.rs`, `i686/encoder/x87.rs`, `x86/encoder/avx.rs`,
  `x86/encoder/mod.rs`, `x86/encoder/gp_integer.rs` (x87 forms, diff
  mov): KEEP + F8. Pop bases match GAS byte-for-byte (bare forms
  identical to the old arms; the old arms silently dropped explicit
  operands — real misassembly, fixed). The DC-base de-swap survived a
  challenge from first-principles x87 semantics: the GAS 2.47
  differential (not the manual) is ground truth, and both archs'
  `fsub %st,%st(4)` = DC E4 rows pass. `parse_st_num` range-validates
  (no `base+n` overflow; `st(8)` pinned rejected). `movq $(A-B)` C7/PC32
  form verified arithmetically (P-cancellation holds for imm32 loads)
  and differentially, including the kernel's same-section spelling
  (new F9 rows). i686 test-harness asymmetry (no `apx_tests` helper)
  accepted: the casefile differential covers parser+encoder end-to-end
  on both archs. F8 tightened x86 `encode_x87_pop_reg` to `pub(super)`
  (only the parent calls it; matches i686 and the patch's own
  `pub`→`pub(crate)` direction in `state.rs`).
- `linker_common/eh_frame.rs` (F2 guard + scan tests): KEEP. The
  forward-CIE guard fixes a real underflow (debug panic / release wild
  pointer / FDE→CIE reclassification). The 4 scan tests pin upstream's
  rewritten reader (regression value for the guard's assumptions); the
  compact test is a TRUE guard test — proven by mutation (guard removed
  → FAIL on byte-mismatch; restored → PASS). No interference with
  upstream #643's fail-closed rules (complementary layers).
- `state.rs` (epoch discipline): KEEP. `pub`→`pub(crate)` enforces the
  documented must-not-call; assert message gains the audit vocabulary.
- `x86/codegen/emit.rs` + `x86/codegen/memory.rs` (#642 fold contract):
  KEEP + F12. Deciding-side accept set (13 types + shift ≤ 3 + store
  staging rule) EXACTLY equals the emitters' sets on all four arms
  (FP/dec widened by #642 bit-exactly: D32→movss, D64→movsd; int sets
  identical; I128/U128/F128/Void refused both sides; shift guard both
  sides). Store strictness is the safe direction (refuse-more). Refused
  folds emit nothing (pinned). F12 pins the PhysReg↔name mapping
  (10=r11, 16=rdx, 1=rbx, 3=r13) so a renumbering fails loudly.
- Casefiles (`pc8` ×2, `x87` ×2, `evex-avx512f-dq-bw`, `xop-vpermil2`):
  KEEP + F9. All ACCEPT rows byte-verify vs GAS 2.47 (asmdiff compares
  allocated-section bytes AND the full normalized reloc table — verified
  in source). F9 added the 3 missing diff-mov ACCEPT rows (x64
  same-section fold in kernel spelling + REX.B r15; i686 PC32 movl;
  i686 fold movl): 12/12 x64, 13/13 i686 green on first run. Bonus
  finding: i686 fold uses B8+rd and x64 uses C7, each matching GAS's own
  arch-specific choice (`b9 07 00 00 00` both sides, value 7 exact).
  Per-arch x87 sample differences are shape-complete on both sides —
  not a gap.
- Session docs (25B, S03, S04): KEEP + F13 (S04 addendum, see above).
- `arena_session_restore.sh`, `lccc-snapshot.sh`: see §S05.0 (F2/F3/F14).

Pre-existing issues found and dispositioned (bytes-correct, no action):
`.long` overflow truncates silently where GAS warns (no warning infra in
this assembler — systemic, recorded as TODO); duplicate user labels
overwrite silently (pre-existing module behavior, dot_anchor consistent).

## S05.4: profile findings (worth knowing)

- `fastbuild` inherits `release`: **debug_assertions are OFF** in the
  fastbuild binary AND in `cargo test --profile fastbuild` (proven with
  a `cfg!(debug_assertions)` probe; `cfg(all(test, debug_assertions))`
  modules compile out — 3643 vs 3651 tests). `rustc --test` does NOT
  force assertions on; the profile rules.
- Consequence: F7's assert and the stream-diagnostic tests execute only
  under the `cargo-test-debug-assertions` slow gate
  (`--config profile.fastbuild.debug-assertions=true`) and dev-profile
  runs. The F7 proof combined a panic-probe (path executes) with an
  assertions-on unit run (invariant holds) — jointly airtight.
- A red herring was chased and retracted mid-session (a "stale cargo
  binary" theory for a passing inverted assert); the actual cause was
  the compiled-out assert above. No sandbox staleness bug exists.

## S05.5: gate + benchmark verdicts

- `check_phi_acyclic_order.sh`: GREEN on this tree (exit 0): rot()
  default 55/2 < legacy 72/34 and < local gcc-14.2 71; i686 legs,
  sha256 digests (`ebf7d5612b4881d9`), and switch wiring all pass. The
  #638 hosted shape is fixed upstream by #645's real improvement (not
  by weakening anything); this tree reproduces the adopted state
  exactly. No gate change made (see §S05.2 for the no-gaming rationale).
- Full CI mirror: verdict recorded in the snapshot ledger entry for
  this session (`ci_gate=ci_local-full-PASS@…`, transcript
  `/home/user/ci_full_S05.log`). The tree was content-frozen before
  the stamp; the ledger description carries the gate tally.
- Benchmarks (quiet box, post-CI): H2H A/B re-run on the final tree
  (rot/sha × on/off × x64/m32 + wall + bounds) and the 4 workload
  proxies; results in `/home/user/bench_S05.log` + `/home/user/h2h/`
  refresh. Runtime is the primary metric; the S04 decision thresholds
  (x64 win ≫ rot32 cost, sha32 guard holds) are re-checked, not assumed.

## TODO for future agents (ranked; S04 items carried with status)

1. **`_Decimal32 s = 0` ICE** (`constant_fold.rs:832`) — pre-existing,
   still open. Repro in S04 notes.
2. **Pressure-aware reassociation** — lag trees win x64 (+21%), the
   root-fold tree wins rot32 (+14%); build the selector. Evidence:
   `/home/user/h2h`, S04 §TODO2.
3. **rot +2 static gap to pristine GCC 13.2** (57 vs 55): loop-shape
   selection (pointer-bump form, callee-saved minimization). Runtime
   already wins; this is polish. New in S05.
4. **Assembler warning infrastructure** (`.long` truncation, x87
   "translating to", …): bytes match GAS everywhere probed; only
   diagnostics are missing. Systemic project, not a one-off. New.
5. **Expat/SQLite hot-path profiling** before any codegen change
   (S04 §TODO3; oracle gap survey `/home/user/oracle-rank4.json`:
   lccc leads put_varint −22 and name_length −8, trails varint-get +6
   and init-heavy mains — profile, don't chase statics).
6. **TBM + vfpclass encoders** (S04 §TODO4, evex-fp16 items 6/2).
7. **fb-label diagnostic** (S03-TODO1, S04 §TODO5).
8. **eh_frame_hdr reservation over-count + linker-oracle differential**
   (S04 §TODO6; `tools/linker/`, needs provisioned oracles).
9. **Full TODO-harvest** across all ~45 follow-up docs (S04 §TODO7).
10. **Re-verify on Ubuntu 26.04** post-2026-10-19 migration (S04 §TODO8;
    margins re-check against the corrected ruler).

## Evidence index

- `/home/user/ms178-1.patch` + `/home/user/artifacts/` (ledger, series,
  bundle `bundle_clone=OK`): the deliverable.
- `/home/user/ci_full_S05.log`: full-CI transcript (mode=full).
- `/home/user/bench_S05.log`, `/home/user/h2h/`: benchmark reruns.
- `/home/user/oracle-rank4.json`: 4-proxy gap survey (S05 tree).
- `/home/user/rebased-delta.patch`: pre-fix rebase delta (audit input).
- `/home/user/rot-mine.txt`, `/home/user/rot-gcc132.txt`: rot diff pair.
- `/home/user/build_fast_S05.log`: zero-warning fastbuild transcript.
- `/home/user/restore_S05.log`, `/home/user/upstream-clone.log`: recovery.
