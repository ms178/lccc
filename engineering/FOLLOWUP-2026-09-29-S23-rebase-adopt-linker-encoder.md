# S23: rebase onto adopted upstream linker+encoder work, NOTYPE port, validator+hardening fixes (2026-09-29)

## The rebase: main unmoved, new work on descendant branches

`main` is still `93f2a43b` (ls-remote authoritative, twice). The "merged new
work" is two UNMERGED descendants, both landed hours before this session:

- `arena/01a0ea47` `be9e349c` (2026-09-28 23:11Z): x86/i686 encoder —
  monitor-family matrix, string-op segment scan, `movsl (mem),(mem)` arm,
  imul/test.s/data16 fixes + 4 casefiles. Zero file overlap with our patch.
- `arena/01a0ea7d` `1b1b5250` (2026-09-29 00:10Z): linker — DefsymPlan order
  semantics, plain GOTPCREL relaxation, PIE validation, PLTOFF/copy rules,
  i686 on-demand copies, ld search paths + 749 lines of linker tests. Eleven
  files overlap ours; its i686 on-demand restructure reopens the exact
  NOTYPE-code hole S22 closed (raw `!is_func` admits code to `needs_copy`).

Both adopted as base via `cherry-pick -x` (authorship preserved; when they
merge to main the picks drop out as empty). Merge order main+47+7d: the two
are file-disjoint, applied clean. `ms178-1.patch` stays based on main, so it
carries the adopted commits too (attributed per-commit in `series/`).

## Adopted-base audit (before trusting it)

- `defsym.rs` (585 new lines): no non-test unwrap/expect/panic; all indexing
  over internally-valid statement ids; `index: i+1` 1-based so callers'
  `st.index - 1` cannot underflow. PASS.
- PIE validation (emit_exec applier): PC32-vs-ABS refusal mirrors lld and
  beats bfd (which silently miscompiles, measured); narrow-absolute check
  mirrors bfd's text. Our COPY/canonical-PLT flows excluded by
  `is_dynamic`/`plt_idx` guards — verified by reading, proven by gates.
- PLTOFF exec/shared rules + GOTPCREL `plain_map0_opcode` (bounds-safe,
  well-tested, deliberately stricter than bfd's opcode-byte-only test):
  reviewed, sound; new sites use raw `STT_OBJECT` (expected — predates the
  classifier; ported below).
- Base standalone: `cargo check` green zero warnings, rustfmt clean,
  `cargo test --lib` 3862/0.

## Our rebase + the port

- Six S22 commits replayed; exactly ONE real conflict (i686 PC32 arm).
  Merged arm: `is_code = is_plt_code_type(...)` takes the PLT on 32-bit
  refs + un-copies; `!is_code && != TLS` copies. Provably preserves 7d's
  typed routing bit-for-bit (FUNC/IFUNC paths identical; only NOTYPE-code
  moves, from COPY to PLT). The `1e7f6518` auto-merge verified site by
  site (field placements, sweep+needs_copy coexistence, predicate spots).
- Port: `copy_data_type` into 7d's three new x86 sites (PLTOFF64 exec arm,
  `pltoff_of_data`, shared-PLTOFF64 refusal) + shared-link doc note.
- Full COPY-vs-PLT site sweep afterwards, all backends: every decision
  routes through the classifier; the `-T` script path verified
  decision-free (constructors only). Wiring audit: all four
  `new_dynamic`/`dynamic_import` constructors pass the reader bit through
  (x86/arm confirmed in code — the earlier worry was unfounded).
- Gate: `check_linker_notype_code.sh` gains x86-64 NOTYPE-data `@PLTOFF`
  (COPY both modes, no JMP_SLOT, functional read, `-shared` refusal
  text). Discriminating: fails on unported code both ways.

## S23 fixes and closed audits

- Validator: memory-memory short-`movs` IS the string op. `rep movsl
  (%esi),(%edi)` assembles (GAS 2.44: `f3 a5`, %ecx counter) but the S22
  gate exempted every `movsl`-with-operands as MOVSX — a missed-%rcx-write
  false negative. `movs_has_register_operand` → `movs_is_movsx`: MOVSX
  iff two operands + pure-register dst (every real MOVSX has one, ISA
  fact; `%ds:(%esi)` correctly counts as memory). 5 new pins, 9/9.
- i686 phdr walks: all three loops checked (`checked_mul/add`, incl. the
  `+32` end) — one is ours, two pre-existing with the same shape.
  `parse.rs` section loop analyzed SOUND (checked table bounds; the only
  unchecked `+32` needs a ~4 GB file on a 32-bit host, unmappable —
  documented, not churned).
- Closed, no change: weak-DSO-data (ours matches bfd exactly — GLOB_DAT,
  no COPY, prints 5; only an UND-entry type-tag cosmetic differs);
  mem-mem `movsb/movsw/movsl/movsq` encoder (byte-exact vs GAS; 32-bit
  `movsq` loud-rejects exactly like GAS).
- Owned prototype bug: first PLTOFF probe used L-GOT as an absolute
  address (segfault) — `@PLTOFF` needs the GOT-base add, as 7d's fixtures
  show. The linker was right in all three observations (COPY, value,
  NOTYPE fidelity).

## Process notes

- NEVER two `edit_file` calls to the same file in one parallel block:
  last-write-wins silently dropped a conflict resolution (caught by the
  marker-count check before amend; resolution re-applied + amended).
- `dl/binutils-2.47.tar.xz` (28 MB) deleted: redundant —
  `ensure_gas_247.sh` fetches its own tarball; nothing references `dl/`.
- Turn-end slimming (cap discipline): delete `.git` (bundle carries it)
  and the rustup binary (re-download per turn; network already required)
  → ~101 MB countable.
- Carryovers: weak-UND dynsym OBJECT-vs-NOTYPE tag (cosmetic); SEC
  hit-window research (still open); bare-`rep movsd` unreachable note.
