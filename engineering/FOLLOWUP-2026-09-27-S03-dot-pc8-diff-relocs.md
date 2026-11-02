# Follow-up: S03 dot/PC8/symbol-difference relocation correctness (G3)

**Base:** upstream `e9f572e` (PR #640 merge; rebased from the S02 base
`a89e685`). This session rebased the worktree onto latest main, finished the
G3 assembler track (`.` handling, 8-bit PC-relative relocations, and
symbol-difference folding/errors), and ported the S01 eh_frame F2 guard onto
upstream's rewritten reader. Patch: `ms178-1.patch` (S03 snapshot).

## Done

### G3.1: `.` (location counter) anchors — both archs

`src/backend/elf_writer_common.rs` (`dot_anchor`, in `encode_instruction`):
- `.` jump targets (`jrcxz .`, `ja .+0x10`), reloc symbols and diff
  subtrahends (`movq $(xtrn - .)`) anchor to a synthetic `.Linsndot_` label
  placed via `place_label`. It rides `shift_after` like any label (so
  relaxation and `.`-arithmetic stay correct) and never reaches the symbol
  table.
- Validated byte-identical vs GAS 2.47 on both archs (`E3 FE`, `E2 FE`,
  `E8 FB`, `77 0E`, `48 C7 C0` + `R_X86_64_PC32 xtrn+3`).

### G3.2: size-8 `mov $(a-b)` is C7/0 + PC32, never movabs

`src/backend/x86/assembler/encoder/gp_integer.rs`: `movq $(a-b), %reg` now
emits `REX.W C7 /0` + `R_X86_64_PC32` diff-reloc. Foldable pairs fold to the
imm32 (both signs verified byte-identical, e.g. `48 c7 c2 f0 ff ff ff`);
external-minus-local converts via the diff path. The old movabs +
`R_X86_64_64` was 3 bytes longer for foldable pairs and unresolvable for
the rest.

### G3.3: `resolve_internal_relocations` is fallible; GAS error laws

- Surviving differences are hard errors with GAS's exact text and naming
  law (a local minuend folds to its section, everything else stays
  verbatim): `can't resolve .text - xtrnA`, `can't resolve xtrn1 - xtrn2`,
  cross-section both directions (12/12 message combinations match GAS 2.47
  on both archs, including weak symbols and numeric labels).
- New 8-byte conversion rule: `.quad xtrn - local` (same-section
  subtrahend) becomes `R_X86_64_PC64` (the old path emitted a bogus ABS64,
  evaluating S+A instead of S+A-P).
- Out-of-range short-only branches error with GAS's exact format, including
  the address-width law: positive values decimal, negatives as hex of the
  address cell, offset 16 digits on 64-bit / 8 on 32-bit
  (`value of fffffed1 too large for field of 1 byte at 0000012e` on i686).
- `Result<(), String>` plumbed through `relax_jumps`, `patch_short_jumps`,
  `resolve_deferred_byte_diffs`, and the finalize path. The old
  `patch_short_jumps` assert (assembler panic on out-of-range) is now a
  clean diagnostic.

### G3.4: PC8 relocations for external short-only targets — both archs

- x86-64: surviving `PC8_INTERNAL` relocs rewrite to real `R_X86_64_PC8`
  (`externalize_pc8_reloc`; the INTERNAL number used to leak and truncate
  to `R_X86_64_64`, an 8-byte patch over a 1-byte field).
- i686 / x86-64 `.code32`: the encoder records no reloc for jecxz/loop, so
  `patch_short_jumps` pushes `R_386_PC8`/`R_X86_64_PC8` itself (addend =
  `target_addend - 1`, wrapping; the existing-reloc scan doubles as the
  already-pushed guard across the two relax passes). REL bake yields the
  GAS-identical `0xFF` disp byte; addended externals (`jecxz ext+5` -> `e3
  04`) match too.

### G3.5: i686 `.quad` survivors are unrepresentable (GAS errors)

`unrepresentable_quad_error`: on targets without a 64-bit reloc type,
surviving 8-byte relocs error exactly like GAS — `.quad ext` (and `.quad
ext+off`) with `cannot represent relocation type BFD_RELOC_64`, `.quad ext
- .` and cross-section `.quad` with `..._BFD_RELOC_64_PCREL`. Gated on
`reloc_pc64() == reloc_pc32()` / `reloc_abs64() == reloc_abs(4)`, so x86-64
behavior is unchanged by construction (audited: no i686 instruction reloc
ever carries patch size 8; there is no `.reloc` directive to inject one).

### Tests

- `tests/asm-diff/pc8.casefile` (10 groups) and
  `tests/asm-diff/i686/pc8.casefile` (11 groups): success differentials
  plus `reject` groups, all oracle-green vs GAS 2.47 (also with the exact
  CI oracle invocation for i686).
- 3 unit tests pinning the exact error strings (both archs, both signs,
  both BFD messages).
- Red-team probes (all GAS-identical): weak-symbol diffs, numeric diffs,
  cross-section `.quad`/`.long` in both directions, folded mov-diffs in
  both signs, `.quad ext+off`.

### Rebase onto `e9f572e` + S01-F2 port

- The reboot recovery put `.git` at latest main while the worktree was
  S02-era; 18 files were pure reverts of upstream `a89e685..e9f572e`
  (#639, #640, two-lane, CFI) and were restored to upstream. The map-i64
  gate wiring came back with `scripts/ci_local.sh`, fixing the
  `ci-gate-parity` failure.
- S01-F1 (`eh_record_extent`) is fully subsumed by upstream's
  `record_bounds` (same three bullets: truncation, sub-id_size, wrap) and
  was dropped. S01-F2 (compaction underflow on forward `CIE_pointer`) is
  NOT subsumed: upstream's `compact_eh_frame` still computes `field_pos -
  new_start[cie]` unguarded. Proven with a failing test first (`attempt to
  subtract with overflow` at `eh_frame.rs:624` under
  `-C overflow-checks=on`), then the 3-line guard was ported. The ported
  test uses an extended-format FDE plus a pruned record between FDE and
  CIE: the original S01 test shape is vacuous on the rewritten reader (a
  32-bit forward pointer wraps `cie_pos` into unmapped `u64` space and
  exits via the dangling-pointer path before reaching the subtraction).
  S01's 4 scan-hardening tests were also ported (they pin `record_bounds`
  behavior upstream's suite does not cover). 19/19 eh_frame tests pass in
  both normal and overflow-checks profiles.

## TODO for future agents

1. **Undefined fb-label diagnostic (all branch kinds).** `jmp 1f` / `jrcxz
   1f` / `jecxz 1f` with no `1:` defined: GAS errors (`local label '"1"
   (instance number 1 of a fb label)' is not defined`), lccc emits an UNDEF
   reloc against the literal `1f` and fails later at link. Sound (no
   miscompile) but late and noisy. Needs the instance-number computation;
   fix uniformly for growable and short-only branches, both archs.
2. **Undefined-numeric diff message.** `.long 9f - ext` (no `9:`): GAS
   double-errors with an internal `.L9\2021` spelling leak; lccc says
   `undefined label in .byte diff: 9f` (pre-existing, clearer). Both
   refuse; consider a `reject` casefile group to pin the refusal.
3. **`.reloc` directive.** The assembler has no `.reloc` support; the G3.5
   audit relies on that (all patch-8 relocs are writer-generated). If
   `.reloc` is ever added, re-audit `unrepresentable_quad_error` for
   user-specified (type, size) pairs.
4. **Forward-CIE guard direction.** For the vanishingly rare pruned-between
   forward-FDE input, skip (current, S01 contract) keeps a stale pointer
   while wrapping would rebase it. Valid inputs never fire the guard either
   way; revisit only with a consumer-side (libgcc/unwinder) forward-pointer
   tolerance study.
5. **Oracle coverage.** `pc8.casefile` (x86-64) is validated by direct
   asmdiff runs like the other x86-64 corpora; only
   `merged-pr629-followup.casefile` is CI-gated for x86-64. The i686
   `pc8.casefile` IS CI-gated (full-corpus `i686-asm-diff` gate).
