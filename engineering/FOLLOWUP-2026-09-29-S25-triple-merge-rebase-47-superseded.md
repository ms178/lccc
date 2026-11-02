# S25: rebase onto triple-merge main; 47 superseded by PR #675 (2026-09-29)

## The wipe, the restore, the rebase

Mid-turn wipe removed `/opt`, `.git`, `target/` and swap; snapshot #23
survived (bundle sha `1de807be…` matches the ledger). Restored in order:
bundle → `.git` (head `ceee0b70`), rustup to `/opt`, swap (8G) +
`gcc-multilib g++-multilib libc6-dev-i386 python3-yaml`, then fetched
immediately. Mode-bit churn from the wipe (181 files 755→644) reset with
`git checkout -- .` before anything else — never resolve conflicts on a
tree that has phantom diffs.

`main` = `800b439f`, delta over S24's base is SIX commits / three PRs:

- #673 `8717a7a9` — the GOT64 fix we ADOPTED as `7c2e3023`: now upstream,
  our pick correctly dropped (content identical, `git diff` between the
  two = empty, proven before the rebase).
- #674 `6b8e14c0` — vectorizer: omit provably-dead scalar remainder
  loops + causal slot census. Touched `ci_local.sh`/`ci.yml` (both-added
  gate conflicts with our gate wires) and no file we own otherwise.
- #675 `bf7ac1c1` — "string-op slot law, PC16 writer plumbing, GOTPC
  addends, @PLT/addr16/xchg.s laws": a follow-up to PR #671's
  operand-direction round, audit-driven, byte-probed against live GAS
  2.47.20260726 (their gates: asmdiff 1316/1316 + 602/602, clippy -D
  warnings, 3857 unit tests). Touched the SAME encoder files as 47.

## The central decision: 47 is superseded — dropped entirely, with proof

`arena/01a0ea47` `be9e349c` (47, the encoder adoption we replayed as
`31e3f16b`) conflicted in six files. Before touching a marker, the
supersession was PROVEN, not assumed:

1. Reverse-apply test (`git diff 1d72c041 47 | git apply -R --check`):
   four of ten files already in main outright; the six that failed need
   semantic review.
2. Cherry-pick partial state: `git diff HEAD` showed ONLY conflict
   content — every non-conflicting 47-hunk was a no-op (already in
   main). Zero real content would be lost.
3. All 8 code hunks reviewed one by one; main's side won every time:
   - data16 `@PLT` rejection (x86 gp_integer ×3): #675 hoists the check
     BEFORE the opcode push via `strip_plt_suffix` + `sym.len()!=…`;
     47 pushed `0xE9` first and only then rejected — partial `self.bytes`
     state. Strictly better on main.
   - string-op classification (i686 mod ×2): #675's shape law
     (`movs/cmps => all_memory`, `lods/stos/… => !vec && (all_mem ||
     acc_shape)`) fixes exactly F1 of their audit — 47's coarser
     `!has_vector_reg` rule let the MOVSX alias `movsb %es:(%edi),%eax`
     take the string law and drop the segment. Better on main.
   - GOTPC addend (i686 mod): main does `addend += offset-start`
     (preserves AND shifts any future nonzero base); 47's
     `if addend==0 { addend = … }` preserves without shifting — wrong
     for nonzero creators. Better on main.
   - `movsl (mem),(mem)` 0xA5 arm: main carries it in the refined
     dispatch (`"movsb"|"movsw"|"movsl"` at :1285, `0xA5` at :1311) —
     47's standalone arm is subsumed.
   - import-list hunk: main = 47's list + `is_size_matched_accumulator`.
4. Casefiles: every single line 47 added (29+12+49+4) is present in
   main's versions; main's carry more (their message: 80 probe shapes).

Resolution: `git checkout HEAD --` on the six files → tree byte-equal to
main → the pick went empty and was skipped. 47 is upstream in spirit and
stricter in fact; replaying it would have REGRESSED five laws.

## The rest of the replay

- Gate-wire conflict (ours vs #674's `vec-dead-remainder`) resolved as
  the union in BOTH `ci_local.sh` and `ci.yml`; `bash -n` + YAML parse
  verified in the same breath as the marker-count check.
- Remaining 11 S22/S23 picks + 3 S24 picks: ZERO further conflicts.
  Result: 14 commits on `800b439f`, tree clean, `cargo check` green
  zero warnings, `cargo fmt --check` clean, fastbuild 0 warnings
  (3m18s), and the five targeted gates (notype, copy-alias,
  got64-old-spelling, tls-pie-preemptible, upstream tls-model-selection)
  all PASS — the behavior proof that neither supersession nor the
  union-resolution lost anything.

## Validation & carry-forward

- `ci_local.sh --fast` on the final tree (see ledger entry 24).
- #675 never touched `x86/linker/*` (its stat is assembler +
  `elf_writer_common` + casefiles only) — zero interaction with our
  S22/S23/S24 linker work beyond the clean-apply proof.
- PR663 doc §5 statuses inside main are #673's originals (open for 5.1,
  as-written for 5.2/5.3) — our §5.1 fix sits ON TOP and is still the
  current resolution; our S24 doc's ledger claims remain accurate.
- Still true: no `LCCC_ASSEMBLERS` old GAS in CI → our
  `.reloc`-synthesis gate is the only CI cover for the ≤2.43 spelling.
- TODOs unchanged from the S24 doc (SHF_TLS unification, non-TLS
  GOTTPOFF refusal, gate promotion into the suite, SEC drop rationale).
