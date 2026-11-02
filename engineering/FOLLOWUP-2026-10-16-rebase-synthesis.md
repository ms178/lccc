# Follow-up — 2026-10-16 session: rebase synthesis, PR triage, harness completion

Scope: re-base of the S01 session patch onto main `19be25d` (merge of PR #840),
red-team triage of all open PRs, repair of every locally failing gate, and
completion of the sandboxed oracle harness. `ci_local.sh --fast` green is the
session's exit criterion; branch `arena/ff2b91ca-lccc`.

## Accomplished (validated)

### Rebase + merge-regression repairs
- Session patch (312 files) re-synthesized onto `19be25d`; 41 conflicted
  files resolved hunk-by-hunk (decisions logged in the commit messages).
- Four wholesale-pick casualties found by DIFFING BOTH SIDES, all repaired
  and pinned by tests:
  1. riscv `checked_auipc_delta` range proofs restored at 4 sites while
     keeping the patch's new `R_RISCV_32` shared-object guard
     (relocation contract 818/818).
  2. `loop_rotate::loop_exit_threshold` whole-trajectory wrap certificate
     re-ported — fixes a real `-fwrapv` miscompile (step `2^30-1`,
     `affine_control_proof.c`: 95 vs oracle 97); unit test
     `refuses_a_wrapping_controlling_expression` pins it.
  3. note-run oracle consolidated on the section-aware
     `tests/corpus/test_linker_note_runs.py` (ci.yml + ci_local.sh).
  4. `[AFFOLD]` debug grammar aligned across compiler, gate regexes and
     test stubs (two drifted sites: `check_affine_loop_fold.sh` survival
     assertion still grepped a `[ROT] ... folds:` form that can never
     match; the stub printed no `[AFFOLD]` prefix).

### Affine fold switch topology (the design decision of this session)
`hot_guard_64` proved foldable by BOTH producers, which made the previous
"independent switches" model self-contradictory across three gates. Final
semantics, pinned by `check_unsigned_affine_loop.sh`'s 2x2 matrix,
`check_affine_exit_compare.sh` and `check_pr824_boundaries.py`:
- `CCC_NO_AFFINE_EXIT_FOLD` kills EVERY affine exit-compare producer
  (a kill switch that leaves one producer running turns the A/B that
  condemns the fold into the A/B that hides it).
- `CCC_DISABLE_PASSES=loop_bound` removes only the early unsigned
  canonicalizer for attribution; the late fold takes over where its
  weaker proof allows.

### Open-PR triage (all four audited against this branch)
- **#838 ABSORBED** (`5861118b`): prefix-aware control-transfer decoding +
  far-jump conservatism in vex_promote's CFG model; wide-call contract
  split structural/native; temp_files lease docs; callgrind receipt
  retention (fail-closed). 24+78+15 unit tests, 42 contract tests.
- **#825 ABSORBED** (`63c95ed7`): AArch64 visibility/ADR linker soundness,
  `#APP`/`#NO_APP` provenance via `ArchCodegen::asm_block_markers` (ARM
  spelling unified to `#APP`), `-o -` GCC parity, foreign-intrinsic
  rejection. 5 gates wired in ci_local.sh AND ci.yml (parity 243).
- **#829 REJECTED**: earlier generation of the same lineage; would remove
  FXSAVE/XRSTOR conservatism, strict symbol-addend parsing (its
  `unwrap_or(0)` swallows errors ours reject), refresh-count tripwires.
- **#786 REJECTED**: seg_gs feature superseded — main already carries the
  evolved form (seg_copy.rs, 32 seg tests, type_builder qualifiers).

### Harness (sandbox has SNI-filtered egress; Actions-tag transport)
Working oracle set, all verified: gcc-16 16.2.0 (`/opt/gcc-16`), -m32
multilib, GAS 2.47 x86_64/aarch64/riscv64 (`~/.cache/gas-2.47-*`),
ld.bfd 2.47 (`~/.cache/ld-2.47`, NOT system-default — see below), mold
2.42.1 + `ld.mold` shim, valgrind 3.19.0, qemu-user 8.2 aarch64/riscv64/
i386/arm (tag `harness-cache-3`), ARM A64 ISA XML 2026-09 (sha-pinned,
tag `harness-cache-2`), kernel relocs tool v6.12 (`~/tools/bin/relocs`).
Harness lessons encoded into the repo:
- `setup_kernel_tools.sh` adopts pre-placed sources (same offline pattern
  as `ensure_gas_247.sh`/`ensure_arm_a64_xml.sh`).
- `gotpcrel_edges` falls back to a pinned `ld.bfd` (`LCCC_BFD_LD` or
  `~/.cache/ld-2.47`) through a `-B` shim when the distro ld cannot link
  CODE_4_GOTPCRELX, else SKIPs honestly.
- `test_benchmark_input_contracts.py` probes that its GCC can LINK
  `-fsanitize=undefined` (a from-source gcc-16 built
  `--disable-libsanitizer` otherwise fails with `-lubsan`).
- Do NOT install the from-source ld 2.47 as system `ld`: it lacks the
  distro sysroot search paths and flips dtag defaults (breaks
  `lccc_ld_default_search_path_x86_64`, `text_relocations`, i386 tags).

### Gate scoreboard
- linker suite: 304 pass / 0 fail / 0 skip (bfd + mold oracles).
- pr824 boundaries with --cross: 265/265 (qemu-backed arm/riscv rows).
- full cargo test: 4914 passed / 0 failed.
- `ci_local.sh --fast`: all gates green as of this session's final run.

## To-do for future agents
1. **Remove the TEMP transport workflows before merging to main**:
   `toolchain-cache.yml`, `harness-cache.yml`, `harness-cache2.yml`,
   `harness-cache3.yml` (keep the orphan tags `rust-cache-1.97.0`,
   `harness-cache-1/2/3` — they are the sandbox restore path).
2. Godbolt-oracle codegen comparison vs GCC 16.2 / Clang 23.1 / ICX on
   benchmark-corpus / zlib-ng / gzip / expat shapes: the godbolt API is
   unreachable from the sandbox; use the local `scripts/oracle_asm.py` +
   GAS/ld 2.47 + gcc-16 pipeline instead (all pieces now provisioned).
3. Sandbox turn-boundary wipes persist ONLY /home/user (minus caches):
   re-run the restore recipe (swap, /opt/rust from tag, /opt/gcc-16,
   dpkg debs, mold, qemu, as/ld symlinks, pyyaml) at every session start.
4. i686-only mold build preset (user preference) — not yet needed because
   the transported mold binary suffices; revisit if mold must be rebuilt.
5. `-fwrapv` `.flags` files on main dropped `-O2` in several tests
   (test-strength regression risk) — audit remaining `.flags` diffs.

## Session 3 addendum (2026-10-16, PR #842 synthesis)

### What happened
PR #842 (`3349eab0`, 288 files, competing with merged PR #841) was
cherry-picked onto main `629cc7d6` and synthesized per-seam (39 conflicts,
every resolution justified in the commit message of `5644af89`). Follow-up
commits: `197a965` (evidence aliases + checked adjusted_limit + census),
`749e14d` (gate-seam repairs incl. two gates that pinned #842's rejected
affine-switch topology), `3b8185f` (W2 load+sext-cast fold restoration).

### Key decisions (agree/disagree record vs both PRs)
- ADOPTED from #842: PassPolicy in IrFunction (semantics.rs deleted),
  byte_count_normalize + strict_recip vectorizers, loop_bound scheduled()
  extraction, first_failing_iv checked math, signed_i32_defs + unified
  3-set prologue helper, per-candidate FP policy fence, evidence tooling
  unification, workflow timeouts.
- KEPT from main/#841: global CCC_NO_AFFINE_EXIT_FOLD owner topology,
  iv_widen/iv_strength_reduce -fwrapv production logic (excised #842's
  signed_unit_header restriction that broke HEAD-pinned admissions),
  inline-asm marker framework, dynamic gcc_versions_at, whole-loop FP
  stability fence (now IN ADDITION to the per-candidate policy fence -
  fesetround-class redteam protection retained at zero general cost).
- NOT ported: #842's two iv_widen tests
  (`wrapping_signed_strict_runtime_bounds_need_clean_unit_step_headers`,
  `wrapping_signed_nonnegative_mask_carries_a_range_into_addition`): the
  first pins the excised signed_unit_header production block (its expected
  rows are wrong under main's stronger widener); the second's domain is
  covered by main's mask-range tests + the adopted wrapv_runtime_iv
  runtime differential matrix (check_affine_loop_fold.sh).

### New regression class found by corpus A/B (fixed in 3b8185f)
Convention/admission splits: #842 moved widening-cast sources out of
needs_sext_values (correct), but the W2 load->cast fold admission still
gated on that marking -> fold silently died (2 insns instead of 1 on hot
rbtree/hash shapes), zero correctness signal. Lesson: when a convention
set changes meaning, grep EVERY consumer of the set, not just emitters.
Gate: tests/regression/check_load_cast_fold.sh (ci_local + ci.yml).

### A/B evidence (this session)
- bench_kernels (15 kernels, -O3): all timed functions bit-identical to
  main; timing deltas = VM noise (equal-hash +-12%). Harness bug fixed:
  links now probe -no-pie (PIE-default hosts measured NOTHING before).
- 74-program corpus asm A/B after fix: 5 diffs, all improvements
  (2x removed redundant movslq, 3x movslq->movl, cltq short form, sieve
  newly YMM-vectorized: 39.1ms vs main 41.6ms vs gcc-16 42.4ms best-of-5).
  Runtime outputs identical to main AND gcc-16 -O2 on every differing
  program.
- Final ci_local --fast: 274 passed / 0 failed / 6 skipped (--fast-only).

### Provisioning additions for the restore recipe
- binutils 2.47 built from the harness-cache-1 tarball ->
  /opt/binutils-2.47 (+ aarch64 cross build -> /opt/binutils-a64);
  pinned paths: ~/.cache/gas-2.47-{x86_64,aarch64}-linux-gnu/bin/{as,objcopy,objdump},
  ~/.cache/dl/binutils-2.47.tar.xz, ~/.cache/lccc-kernel-tools/bin/relocs
  (compile relocs_common.c+relocs_32.c+relocs_64.c with -I. and
  tools/le_byteshift.h), ~/.cache/arm-isa-a64-2026-09/, /usr/local/bin/ld.mold
  symlink (gcc -fuse-ld=mold fails without it), debs-bookworm multilib.
- /home/user persists ONLY the repo; /home/user/artifacts does NOT
  survive turn boundaries. The durable state is the pushed branch.
