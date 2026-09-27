# Follow-up: S06 rebase onto 705de1ad (#646) + F15 staging-rule fix

**Base:** upstream `705de1ad` (PR #646, FP16-EVEX audit response; session
started from the S05 tree at `267a4518` on `e265e05`). This session
recovered from a second turn-boundary wipe (bundle path worked this time
— see §S06.0), rebased onto latest main, root-caused the 2 red gates the
S05 full CI found (90 pass / 2 fail), fixed them with one principled
change (F15), proved the fix six ways, re-ran the full CI mirror green,
and re-benchmarked. Patch: `ms178-1.patch` (snapshot entry `S06-…`,
base `705de1ad`).

## S06.0: wipe recovery (the F2/F3 machinery pays off)

Turn-boundary wipe #2: `.git`, `target/`, processes, swap, shim links
all gone again; `/home/user` files intact. Recovery via
`arena_session_restore.sh`, fully automatic:
- `.git` restored FROM THE BUNDLE (HEAD `267a4518`, original hashes) —
  the first bundle-path success in three sessions (F3's unshallow +
  test-clone fix validated live). Origin remote re-added by the script.
- Toolchain shims recreated (F2a), only `rustup` had survived — same
  single-hardlink decay, same automatic repair.
- Swap: fresh 8G `/swapfile` (was 0).
- `lccc` rebuilt (fastbuild, -j2, zero warnings).
- **Stale-file trap (new finding):** the turn-end snapshot had persisted
  2 files at pre-c2 bytes (`ci.yml` F1 wording, `memory.rs` F12 asserts)
  while `.git` recorded c3. `git status` (content hashes) is authoritative:
  checked the 2 files out from HEAD, tree == c3 exactly. Lesson: after
  any restore, treat `.git` as authoritative and `checkout` dirty tracked
  files unless uncommitted work is expected. TODO: teach the restore
  script to report this state loudly (F16, filed below).

## S06.1: rebase onto latest main (`705de1ad`)

Upstream advanced by exactly one PR: #646 (`a38609d2`, "FP16 EVEX
encoding fixes from audit response", validated vs GAS 2.47.20260726).
`git rebase origin/main` replayed all 3 S05 commits with ZERO conflicts
(predicted: 289–469-line gaps around every hunk; verified per-file
beforehand after correcting a swapped mod.rs/avx.rs attribution).
- Overlap files (`avx.rs`, `mod.rs`): #646's EVEX/VCVT/COMIS gates and
  my x87 pop forms touch disjoint functions; merge smoke (assemble
  `faddp %st(3)` → DE C3, bare `fsubp` → DE E1, plus `vaddph`/`vaddsh`/
  `vcomish` EVEX probes) exit 0 with correct bytes.
- #646's additions (`fp16-evex.casefile`, `distill_bcst_elem.py`, follow-up
  doc) intact; stray `th.s` stays deleted.
- Delta still 27 files, file list identical (the "26" in S05 notes was a
  miscount: 1 ci.yml + 4 docs + 5 scripts + 11 sources + 6 casefiles).
- Codegen untouched by the rebase: `rot.c` output still byte-identical to
  the adopted state (the 10/10 H2H identity carries over verbatim).

## S06.2: F15 — the bogus store-staging rule (both red gates, one cause)

The S05 full CI (base `e265e05`) finished 90 pass / 2 fail:
`vector-copy-elimination` (double_reduction 137 > budget 135,
spectral_norm 290 > budget 287) and `codegen-quality-gate`
(sqlite_varint stackmem 11 → 12). Reassoc exonerated first (on==off
counts). A pristine-codegen overlay (base versions of emit.rs/memory.rs/
common.rs, 36s incremental build) counted 135/286 — the regressor is my
fold contract, and the P-vs-M `.s` diff convicted exactly three sites,
all store-fed folds with %r11-homed SIB base/index that pristine folds
correctly:
- double_reduction: `movl %edx, (%r11,%rdi,4)` (+2 when refused),
- spectral_norm: `vmovsd %xmm4, 208(%rsp,%r11,8)` (+3) and
  `vmovsd %xmm11, (%r12,%r11,8)` (+1).
My override refused them per its {%rdx, %r11} store-staging rule — a set
copied from `const_offset_fold_reg_base_ok` without verifying the
indexed path uses it. The completing audit proves it does not: the whole
indexed-store staging chain (`operand_to_rax` all arms,
`emit_imm_to_gpr`, all 231 lines of `value_to_reg_inner`,
`fp_store_value_xmm` all arms) emits zero %r11/%rdx/%rcx WRITES
(%rax-only, %xmm0-only, one %rcx READ). %rax is unallocatable and SIB
homes are never XMM, so no clobber is reachable; i686's rule stands on
i686-only evidence (accumulator + %ecx scratch, documented clobbers).
Fix: delete the rule (keep the TRUE mirrors: shift ≤ 3, 13-type set),
flip the unit test into an acceptance pin, and document the false
premise in the override so it is never re-added. Six proofs:
135/286 counts, `.s` byte-identical to pristine on both programs,
runtime outputs F15=pristine=gcc on both, vec-copy-elim gate 47/0,
codegen-quality green (the sqlite spill was pressure fallout from the
same refused folds), differential 57/57, benchmark-output-oracle 204/204.

## S06.3: gate + benchmark verdicts

- Full CI mirror rerun on the F15 tree: verdict recorded in the snapshot
  ledger entry (`ci_gate=ci_local-full-PASS@…`, transcript
  `/home/user/ci_full_S06.log`). Tree content-frozen before the stamp.
- Benchmarks (quiet box, post-CI): `/home/user/bench_S06.log` via the
  S05 harness (rebuilt binaries; H2H A/B + compile-time). Results and
  oracle margins in the ledger description.
- Unit tests: `cargo-test` + `cargo-test-debug-assertions` gates cover
  the flipped F15 test and all prior pins (F7b, F12, contract suite).

## TODO for future agents (ranked; carries S05's list)

1. **`_Decimal32 s = 0` ICE** (`constant_fold.rs:832`) — pre-existing,
   still open.
2. **Pressure-aware reassociation** (S05 evidence stands).
3. **rot +2 static gap to pristine GCC 13.2** (57 vs 55).
4. **Assembler warning infrastructure** (`.long` truncation etc.).
5. **Expat/SQLite hot-path profiling** before codegen changes.
6. **TBM + vfpclass encoders.**
7. **fb-label diagnostic.**
8. **eh_frame_hdr reservation over-count + linker-oracle differential.**
9. **Full TODO-harvest** across follow-up docs.
10. **Re-verify on Ubuntu 26.04** post-2026-10-19 migration.
11. **F16 (new):** restore script should loudly report worktree-vs-HEAD
    drift after `.git` recovery (the §S06.0 stale-file trap) instead of
    leaving it to `git status` literacy.

## S08 session (2026-09-27): restore recovery + F31 decimal-const landing

### S08.0 restore recovery (READ FIRST after any wipe/restore)
The snapshot restore drops two things the ledger does not mention:
`.git/` (32M, small!) and `~/.rustup/` (whole toolchain). `~/.cargo/bin/`
keeps only the `rustup` shim, minus its `+x` bit (all worktree `+x` bits
are stripped too). Recovery, in order:
1. `chmod +x ~/.cargo/bin/*`; `rustup toolchain install stable --profile
   minimal -c rustfmt -c clippy` (net required; ~11s).
2. `cd /home/user/lccc && git init -q . && git fetch -q
   /home/user/artifacts/lccc.bundle 'refs/heads/*:refs/heads/*'
   'refs/tags/*:refs/tags/*' && git reset -q <head>` — mixed reset keeps
   the worktree; NEVER `checkout` a branch (destroys uncommitted work).
   Verify base identity first: checkout the bundle head in /tmp and
   `diff -rq` against the worktree (S08: 21 paths = exactly the stack).
3. Restore modes: `git diff --numstat`, `checkout --` the `0 0` (mode-only)
   files. S08: 170 mode-only, 21 content.
4. `git config user.name/email` (repo-local; copy from `git log -1`).
F16 (from S06): the restore script should automate/report all of this.

### S08.1 F31 quantum campaign (the 8 rod divergences)
`check_decimal_const_agrees_with_gcc.sh` compared 69 cases; its python
regexes required TAB-indented `.long` (lccc emits 4 spaces) so the rod
block had never actually run — fixed (`[ \t]+`, dynamic `ROD-TOTAL`).
First real run: 8 divergences, 3 root causes, all in `decimal.rs`:
1. **Trailing-zero strip (round_to_prec + parse):** `1.0000005DF` folded
   to `1` (!!), `10000000` (int AND `10000000.DF`) to `1e7`. GCC keeps
   the rounded/written quantum verbatim (`1000000e-6`, `1000000e1`,
   `1.50DF -> 150e-2`). Fix: delete both strips. Single rule now: never
   strip, never pad; round iff len > prec.
2. **Missing overflow rescue (encode_fields):** `1e91DF` gave Inf; GCC
   fills the coefficient (`10e90`). Fix: `while len < prec && e > emax`
   push zeros, then Inf only if still past the cap. Cap probes confirm
   GCC canonicalizes at Emax-(prec-1): D32 `1e96->1000000e90, 1e97->Inf`,
   D64 `1e384->1e15e369, 1e385->Inf`, D128 `1e6144->1e33e6111,
   1e6145->Inf`, `9.9999999e96DF->Inf` (round-up overflow). The old test
   pins encoded the buggy behavior; all updated + cap pins added. The
   `emax 6111` D128 comment was misleading (IEEE Emax is 6144; 6111 is
   the canonical cap) — comments corrected at all widths.
3. **Float-source quanta (binary_to_decimal_digits + const_to_bid):**
   measured GCC: strip exact expansion, round iff > prec, then pad
   nonzero shorts to MIN 2 digits (`0.5->50e-2, 7.0->70e-1, 100.0->10e1`,
   same at DD/DL); zeros (incl `-0.0`) carry `e=-1` at ALL widths
   (probed DF/DD/DL); NaN canonicalizes to +NaN even from `-nan("")`
   (Inf keeps sign). Fix: pad loop in `binary_to_decimal_digits`,
   `Zero -> enc(neg, &[0], -1)`, `neg && !is_nan` in `enc_special`.
   Literals and ints keep minimal quanta (no pad): `5.DF->(5,0)` and
   int `1->(1,0)` probed. Caveat (open): direct-conversion assumed —
   if GCC routes DF via DD, adversarial double-rounding cases could
   exist; none found, needs a dedicated search to exclude.
Battery now 92 rod cases + hang pin + runtime triangle: **PASS=94 FAIL=0**
(bit-exact vs GCC 14 incl. subnormals, caps, inf/nan, -0).

### S08.2 validation (this tree, commit 2a1c0a47)
- `cargo build --profile fastbuild`: zero warnings (fixed 3 errors +
  2 warnings in new code: by-value match arms, `pub(super)` visibility,
  paren lints). NOTE: never issue two `edit_file` calls to the SAME file
  in one parallel block — last-write-wins silently dropped a fix.
- `cargo test --lib`: 3652 passed, 0 failed; decimal subset 19/19.
- Battery (both copies): PASS=94 FAIL=0. f30test gcc-exit 0 = lccc-exit
  0; f30mix exit 0. New harness case `decimal_implicit_conversions`: PASS.
- Upstreamed: `scripts/check_decimal_const_agrees_with_gcc.sh` (portable
  LCCC/WORK defaults), f30test embedded in `tests/correctness/`
  (f30mix stays workspace-local: GCC rejects it by design).
- Snapshot S08 (`2a1c0a47`, UNGATED — full CI pending per validation
  economy): patch APPLIES-CLEAN, 45 files +4591/-322, deliverable
  `/home/user/ms178-1.patch` (274715 bytes).

### TODO deltas
- DONE here: F31 (G2/G3/G4 + quantum campaign + battery + harness gate).
- NEW: harness `expected_exit_code` (run_correctness.py:39) is unpacked
  but never checked — dead field; wire it or drop it (minor).
- NEW: lccc silently encodes decimal Inf without GCC's `-Woverflow`
  diagnostic (also GCC warns even for rescued-finite 1e96 — arguably a
  GCC bug; do NOT copy that part). Diagnostics parity = future item.
- NEW: decimal-source `-NaN` narrowing (DD->DF) quantum unprobed; signed-
  NaN decimal -> binary/int conversion paths unprobed.
- CARRY: F16/F17/F19/F20/F22/F23 landing; semantic + red-team audit;
  quiet-box S06 bench; final FULL gated snapshot.

## Evidence index

- `/home/user/ms178-1.patch` + `/home/user/artifacts/`: the deliverable.
- `/home/user/ci_full_S06.log`: full-CI transcript on the F15 tree.
- `/home/user/bench_S06.log`: benchmark reruns.
- `/tmp/double_reduction-{P,M,F15}.s`, `/tmp/spectral_norm-{P,M,F15}.s`:
  control/fix `.s` sets (regenerable; `/tmp` is scratch).
- `/var/tmp/lccc-mine-705`: pre-F15 binary (behavioral anchor; scratch).
- `/home/user/dec-battery/`: rod/f30/f30mix sources + binaries; `/tmp/qprobe.s,
  fsprobe.s, tri2.s, tri3.s, cap.s, nanprobe.s, ddz.s`: quantum probe transcripts.
