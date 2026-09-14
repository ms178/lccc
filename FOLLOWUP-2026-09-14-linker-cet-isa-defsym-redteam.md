# Linker CET/ISA/defsym perfection + red-team (S03–S04)

Date: 2026-09-14. Work branch `ms178-1-work`, base `05fb9637` (= origin/main
at the time; verified no newer upstream before snapshotting). Snapshots
S03 (`cet-perfection`) and S04 (`redteam-s04`) in `artifacts/`, canonical
deliverable `/home/user/ms178-1.patch` (APPLIES-CLEAN both times).

Starting point: PR #529 (`41092411`, "linker: fix i686 TLS IE-relax #UD;
merge .note.gnu.property; harden defsym") plus the S02 fmt fix. This
session perfected the CET/property-note merge, the `-z x86-64-*` ISA
interface, `--defsym` diagnostics, the i686 ABS-GOT path and the x86-64
TLS-reloc folding table — then red-teamed the result against real binutils
and fixed what the audit found.

## Done (S03: `cet-perfection`)

- `linker_common/cet.rs` — property merge rewritten against binutils 2.47
  source (`elfxx-x86.c::_bfd_x86_elf_merge_gnu_properties`,
  `elf-properties.c`, `include/elf/common.h`):
  range-based AND/OR/OR-AND classification (+ the two COMPAT outliers),
  OR all-zero removal, `-z` creation for CET bits and ISA levels,
  class-aware note stride (16-byte entries on 64-bit, 12 on 32-bit —
  `align_size = elfclass == ELFCLASS64 ? 8 : 4` in `elf-properties.c`),
  12 unit tests. Fixed two bugs the new tests caught: vacuous-AND on
  empty input (`0xffff_ffff` instead of just the `-z` bit) and a 4-aligned
  parser stride that misparsed every 64-bit note after the first entry.
- `linker_common/args.rs` — `-z x86-64-{baseline,v2,v3,v4}` parsing
  (`z_isa_level`, 0 = unset), deferred `invalid x86-64 ISA level` fatal,
  `LinkerArgs::property_link_flags()`; removed the non-GNU `--isa-level` /
  `-march=` lookalikes and the dead `no_undefined_version`.
- `x86/linker/link.rs` — both builtin paths (`link_builtin`, `link_shared`)
  compute `cet_flags` up front (fail fast on a bad ISA level) and warn
  honestly when `--emit-relocs` is requested (only `-T` script links
  implement retention).
- `bin/lccc_ld.rs` — script links derive CET/ISA flags from passthrough
  (`passthrough_property_flags`, now fallible); `-z<kw>` joined form
  handled identically to `-z <kw>` via `handle_z_keyword` (it used to warn
  "not implemented"); `--isa-level`/`-march=` arms removed (honest
  unknown-option warnings now).
- `linker_common/defsym.rs` + `x86/emit_script.rs` — single GNU-verbatim
  renderer `DefsymError::gnu_message(N)` on all four x86 paths (builtin +
  script, 64 + 32 bit); dead `message()` removed; script-path Alias arm
  replaced a non-GNU diagnostic with a documented unreachable `expect`.
- `i686/emit.rs` + `i686/symbols.rs` — reverted the slot-vaddr ABS-GOT
  mislink (GNU puts the VALUE in the slot, no RELATIVE — measured, see
  below); root-cause fix materialises input SHN_ABS addresses at resolve
  time instead. `i686/input.rs` warns on `--emit-relocs` (was silently
  dropped) and a stale "alias-only" TODO removed.
- `x86/assembler/elf_writer.rs` — `is_tls_reloc` += 44, 45, 47, 48, 50, 51
  (APX CODE_4/5/6 TLS forms; the folding-corruption audit). 43/46/49 stay
  foldable (plain GOTPCRELX). `i686/.../elf_writer.rs` table verified
  entry-by-entry against `elf/i386.h` (comment cite only, no change).
- Tests: unit tests for both `is_tls_reloc` tables, `-z x86-64-*` parsing,
  script-defsym counter/wording, i686 ABS resolve, `handle_z_keyword`.

## Done (S04: `redteam-s04` — what the red-team audit found)

1. **x86-64 warn-rule bug (mine, fixed).** S03 fatalled on ANY unknown
   `x86-64-*` keyword. Measured GNU rule (ld 2.44): only a bad `x86-64-v`
   suffix is fatal; every other `x86-64-*` warns (`-z <kw> ignored`) and
   continues. Both parsers (`args.rs`, `passthrough_property_flags`) now
   implement exactly that; tests updated.
2. **Joined `-z<kw>` inside `-Wl,` groups was silently dropped** by
   `parse_linker_args` (same defect class as the S03 driver fix, one layer
   down). The `-z` keyword body is now one shared `apply_z_keyword`
   function with split + joined call sites; covered by unit tests and a
   new end-to-end harness case (which demonstrably FAILS on the S03
   binary — stale-binary incident during development proved it).
3. **Differential harness cases** (`tests/linker/run_linker_tests.py`,
   new section 13 + `expect_prop_note_match` + `prop_note_map` helper):
   6 cases comparing lccc's merged `.note.gnu.property` against bfd's —
   AND/OR merge, OR-veto survival, `-z ibt` rescue (split + joined),
   ISA-level creation, invalid-level rejection. 8/8 green with tag
   `propnote` (incl. 2 pre-existing same-tag cases).
4. **`scripts/arena_session_restore.sh` hardening** (see "Wipe recovery").

## GNU measurements (all on Debian ld 2.44 unless noted)

| # | Claim | Evidence |
|---|---|---|
| 1 | `-z x86-64-baseline` + ISA_1_NEEDED inputs → BFD internal-error abort (missing `case 1:`) | reproduced: `elfxx-x86.c:4052 in _bfd_x86_elf_merge_gnu_properties`; 2.47 source still lacks the case. lccc deliberately ORs the baseline bit instead (documented deviation). |
| 2 | Bad `x86-64-v` suffix → `ld: invalid x86-64 ISA level: <kw>`, rc=1 | v, v1, v2x, v3x, v9 — all fatal with that exact text. |
| 3 | Other `x86-64-*` → `ld: warning: -z <kw> ignored`, rc=0 | x86-64-foo, x86-64-, x86-64-baselineX, X86-64-v3 (case matters). |
| 4 | Unknown `-z` generally warns; known keywords silent | `-z foo`, `-z foo=bar` warn; relro/now/noexecstack/origin/notext/nocombreloc/max-page-size silent. |
| 5 | CET merge 4/4 + ISA creation identical lccc vs GNU | crafted notes via GAS: AND{3,1}→IBT, OR-keeps-v3, veto drops AND, `-z ibt` rescues, `-z x86-64-v3` creates — byte-identical `readelf -n`. |
| 6 | defsym diagnostics verbatim | ``--defsym:1: undefined symbol `X' referenced in expression``; `--defsym:0: syntax error` (counter always 0); `--defsym:1 / by zero` (no colon). |
| 7 | i386 ABS GOT: slot = VALUE (0x64), zero dynamic relocs | `-m elf_i386`, R_386_GOT32 + `--defsym k=100`. (R_386_GOT32X relaxes to `mov $0x64` instead — no slot at all.) |
| 8 | Joined `-z` accepted everywhere | `ld -zrelro -znow` and `gcc -Wl,-zrelro -Wl,-znow` both silent. |
| 9 | APX TLS reloc numbers | `elf/x86-64.h`: 44/45/47/48/50/51 TLS (GOTTPOFF/TLSDESC), 43/46/49 plain GOTPCRELX. |
| 10 | CET entry stride 16 (64-bit) / 12 (32-bit) | `elf-properties.c`: `align_size = ELFCLASS64 ? 8 : 4`. |

## Validation status

- `cargo build --profile fastbuild` ✓, `cargo fmt --check` ✓,
  `cargo clippy --all-targets --profile fastbuild -D warnings` ✓ (exact CI
  invocations, `-j2`, 4G swap at `/swapfile`).
- `cargo test --profile fastbuild --all-targets`: 2797 + 1 passed, 0 failed.
- `ci_local.sh --fast`: 29 passed + 5 initially failed ALL on missing
  32-bit multilib in the fresh sandbox (every failure was an i686 leg
  dying in system headers); after `apt install gcc-multilib
  libc6-dev-i386` all 5 pass in isolation; full `--fast` re-run on S04
  finished 34 passed / 0 failed / 3 skipped — ALL GATES GREEN.
- `run_linker_tests.py --tag propnote`: 8/8 vs bfd (mold/wild absent here).
- Base `05fb9637` verified == `origin/main` (fetch 2026-09-14); no rebase
  needed. Patch `APPLIES-CLEAN` per snapshot self-check, S03 and S04.

## TODO (deferred with rationale, not overlooked)

1. **Warn on ALL unknown `-z` keywords** (GNU rule #4). Needs a per-path
   honored-keyword audit first: the test suite passes `-z noexecstack`
   ×13 (silence there may be CORRECT if lccc unconditionally emits an NX
   stack — verify, don't assume), and the driver itself injects
   `-Wl,-z,noexecstack` (a naive warn-all would nag on our own flag).
   Now that `x86-64-*` routes through `z_ignored_keywords`, extending the
   mechanism is easy; the audit is the work. (Audit input: `-z
   isa-level-report=` is a REAL GNU keyword (2.44 `ld --help`), currently
   unhandled — decide honor vs warn-ignore explicitly.)
2. **i686 `-z` parsing.** `i686/linker/input.rs` has its own arg parser
   that silently drops `-z` (incl. `-z ibt`, which GNU i386 ld honours).
   Either share `apply_z_keyword` or document the gap; the CET merge call
   in `lccc_ld.rs` already covers i386 `-T` links.
3. **Bare top-level `-z` in `parse_linker_args`.** `-z` is only honoured
   inside `-Wl,` groups; a bare argv `-z relro` reaching the builtin parser
   is dropped. (Normal flows always use `-Wl,` or `lccc-ld`, so this is a
   corner, but it should warn, not vanish.)
4. **i686 ABS-GOT end-to-end differential.** Unit test pins resolve;
   GNU side measured (§7); still missing: a `-m elf_i386` link through
   lccc-ld asserting slot bytes (needs the i386 `-T` script harness).
5. **32-bit property-note end-to-end.** The 12-byte stride is unit-tested
   both directions but never linked differentially (32-bit CET input +
   `-m elf_i386 -T` link vs GNU). Same harness as #4.
6. **Rust toolchain location tradeoff.** This session installed to `/opt`
   (keeps the 10k-file snapshot budget for SOURCES; reinstall ≈ 1 min
   with network). `arena_session_restore.sh` policy is the persisted
   `/home/user/.cargo` (survives wipes but eats the file budget — the
   likely cause of the 1334-file eviction below). `/home/user/.cargo/env`
   now honours `CARGO_HOME` so the restore script reinstalls to `/opt`
   transparently when that env file is present. Decide once, then make
   the script match instead of carrying both.
7. **Upstream deviations to keep an eye on:** the baseline-bit OR (GNU
   aborts — if binutils ever fixes the missing `case 1:`, confirm which
   way they go); `x86-64-v5` (fatal both sides today — revisits when
   binutils grows it).

## Done (S05: `review-s05` — PR #529 review findings)

Full verdict in `engineering/REVIEW-2026-09-14-pr529.md` (agree/disagree
with measurements for every item). Code changes:

- Restored `--no-undefined-version` (real GNU flag; S03 wrongly deleted
  it): builtin parser accepts+records (new unit test), `lccc-ld` routes
  it to the accepted-but-not-implemented warning per that file's own
  tier rule. Enforcement stays a documented TODO.
- Removed the fabricated `__bss_end__` (undefined under real GNU ld —
  measured) from the standard-symbol seeder and the defsym allowlist.
  lccc now rejects references to it exactly like GNU.
- New differential harness case `prop_note_z_lam_u48_sets_both_bits`
  pins the measured `-z lam-u48` → U48+U57 grouping. Tag `propnote` now
  9/9 vs bfd.
- Confirmed no-ops (verified, not changed): defsym backtick quoting
  was already byte-exact since S03; `lccc-ld` empty-link exits 1
  cleanly; user-defined `end` wins over seeding under both linkers;
  x86-64 `is_tls_reloc` widening re-confirmed against the encoder's
  REX2/EVEX paths; i686 ABS-GOT re-measured and verified end-to-end
  (slot `0x64`, no RELATIVE).
- Root-caused PR #529's CI red: rustfmt (16 diffs / 7 files), NOT
  clippy — exact-CI clippy on the PR head itself returns rc=0. The
  work branch is clean on both.

Validation: fmt ✓, exact-CI clippy ✓ (rc=0, pipe-free), `cargo test
--all-targets` 2798+1/0, propnote harness 9/9, behavior probes (tier-2
warning text, silent driver accept, `__bss_end__` rejection, link rc=0
after warning). `ci_local.sh --fast` on S04 was 34 passed / 0 failed /
3 skipped — ALL GATES GREEN.

## Wipe recovery (what happened this turn, 2026-09-14)

The sandbox came back WITHOUT: cargo/rustup, swap, `target/`, `.git`
(entirely), 1334 worktree files (10k-file snapshot cap), and all +x bits
— but WITH: `ms178-1.patch`, `artifacts/` (incl. `lccc.bundle` +
`.base_ref` + `.seq` = 2), and the 11 S03 work files byte-intact.

Recovery (all verified): 4G `/swapfile` (outside the workspace, so it
never pollutes snapshots); Rust stable → `/opt` (`CARGO_HOME`/
`RUSTUP_HOME`, minimal + rustfmt + clippy); `git clone lccc.bundle` +
`.git` transplant into the worktree (uncommitted edits reappeared as
exactly the 11 expected modifications); `git checkout --` of deleted
paths only; mode-noise revert; multilib via apt for the i686 CI legs;
`/home/user/.cargo/env` recreated so future shells just work.

`arena_session_restore.sh` now automates the two lessons: bundle-first
`.git` recovery with upstream fallback (preserves branch + snapshot base
commit; old comment claiming the bundle unusable was wrong for `clone`),
and threshold-guarded (>50 paths) evicted-file restoration before the
exec-bit pass. Validated 2026-09-14 on a quiet tree by running the
extracted step-4 snippet verbatim: 60 deleted files → "restored: 60
(of 60)", with a simultaneously-modified doc byte-identical afterwards
(md5 match — modified files are never touched); 5 deleted files →
"left alone: 5 (below bulk threshold)", restored manually after.
