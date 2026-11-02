# S24: rebase onto new main, adopt GOT64 slot-keying fix, preemptible-TLS dynamics (2026-09-29)

## The rebase: main moved, replay clean

`main` is now `1d72c041` (ls-remote authoritative): PR #672, the merge of
`1b1b5250` (the S23 7d adoption) onto `93f2a43b`. The delta is exactly 2
commits, so S23's `d8c408ff` drops out as upstream and the rest replays:
`s24-rebase` = `1d72c041` + `31e3f16b` (47) + 11 ours, every cherry-pick
clean, `git diff s23-rebase s24-rebase` EMPTY. Base for the patch is main,
so the adopted commits ride along attributed per-commit in `series/`.

A third unmerged descendant appeared hours before this session and is
adopted as base as well:

- `arena/01a0ec4c-lccc` `8717a7a9` (2026-09-29 08:35Z, authored by ms178):
  x86 linker GOT64 local-slot keying fix + assembler-spelling matrix test
  + `docs/PR663_CI_REPAIR_AND_FOLLOWUP.md`. Adopted via `cherry-pick -x`
  (`7c2e3023`), auto-merged clean including the 6-line-adjacent
  `types.rs` hunk (verified: our `lib_in_exec` and their 3-tuple
  `LocalSlots` coexist; `cargo check` green, zero warnings).

## Adopted-base audit (before trusting it)

- `got_slot_addr_addend` (elf.rs): returns the addend for GOT64-family
  only, 0 otherwise. `wrapping_add` + negative-addend `as u64` correct.
- Key `(obj, sym, addr_addend)` threaded through insert (plt_got.rs),
  value fill (`S+A`, emit_exec/emit_shared), field (plain slot offset),
  and every lookup. TLS sets reuse the type with addend 0 in SEPARATE
  maps -- no TLS/GOT key collision (each map is one namespace).
- Global GOT64+addend intentionally divergent from bfd 2.42 (`movabs
  $0x0` there too): matching GNU means not fixing it. Recorded at the
  code, correct call.
- The matrix test (`got64_spelling_matrix`) SKIPs only when NO assembler
  builds the fixture -- impossible in CI (default `as` always present),
  so `--strict` is safe. Its one honest gap: CI has no ≤2.43 assembler,
  so the old spelling it was written for is untestable there. That gap
  is closed below, without an old toolchain.
- `emit_script.rs` untouched by the fix and contains no local-GOT logic
  (a `-T` link has no GOT): nothing to port there. i686/arm/riscv GOT
  forms are bias-style (slot holds S, field adds A) with no S+A-slot
  form, so the defect class is x86-64-GOT64-specific.

## New: synthetic old-spelling GOT64 gate (flagship)

`tests/regression/check_got64_old_spelling.sh`: three locals at `.data`
offsets 0/16/32 loaded through synthetic `R_X86_64_GOT64 .data+N` and
compared against their own addresses, `-no-pie` and `-pie`, expecting
`1 1 1`. The old spelling is forced with the `.reloc` directive
(`.reloc . - 8, R_X86_64_GOT64, .data+N`), which modern GAS accepts and
which emits byte-exactly what GAS ≤2.43 produced (verified with
readelf); the `. - 8` is expression-relative to the `movabs` immediate,
so no byte counting can rot. The gate asserts the input spelling first
(three `.data + N` relocs), so if a future assembler stops honouring
`.reloc` the gate fails loudly instead of testing the wrong thing.

Measured bfd 2.44 behaviour on this input: `1 0 0` -- bfd folds the
addend into the field and reads past the slot. lccc-ld answers `1 1 1`.
The gate pins CORRECTNESS, deliberately not bfd parity: matching the
reference linker here would mean matching its miscompile. Mutation
proof: the gate fails (`1 0 0`, exit 1) against lccc-ld built at
`1d72c041` (pre-fix), passes on this tree.

## Filed-defect ledger (§5 of the adopted PR663 doc): all four resolved

- §5.1 GOTTPOFF/GD against a preemptible global -- FIXED with full
  support, better than the proposed refuse-to-link gate (`90ee3259`).
  `preemptible_tls_names` (emit_exec.rs) builds the set once per link
  (PIE, not `-static`, defined, STT_TLS, default visibility, exported
  per the emitter's own `ExecExports` predicate -- the SAME predicate
  that puts the symbol in `.dynsym`, so the dynamic TPOFF64's symbol
  index always exists). Planner, `.rela.dyn` size pre-count and emitter
  all consult the one set: GD relaxes to IE, GOTTPOFF takes the slot
  path, the slot gets a dynamic TPOFF64. Non-PIE, `-static`, hidden,
  protected and unexported shapes are byte-identical to before.
- §5.2 R64 in read-only storage -- CLOSED, no change. Probed: `.so`
  with `movabs $g` in `.text` gives byte-identical warnings, DT_TEXTREL
  and runtime (99) on bfd 2.44 and lccc-ld; the executable path already
  copy-relocates data-R64 out of RO storage; the suite's
  `text_relocations` test pins the RELRO-head placement, TEXTREL
  policy and `-z text` refusal. The proposed emission-time check would
  never fire -- per the F6 lesson (no untestable changes) it is not
  carried. Revisit only if a violating input is ever constructed.
- §5.3 global GOT64+addend -- NO ACTION, by design (bfd emits `movabs
  $0x0` too; compilers only emit addend 0). Agrees with the doc.
- §5.4 PLTOFF64 gate -- CLOSED, already relocation-typed. Verified on
  the merged tree: `is_got64_family` excludes PLTOFF64, so our S22
  `R_X86_64_PLTOFF64` arm (plt_got.rs) is reachable and matches the
  relocation first, consulting symbol type only for the semantically
  necessary data/code decision. The accidental dependency F5 feared
  does not exist. No code change; S22 closed it.

## The trap the 5.1 fix walked into (recorded so nobody repeats it)

The first build planned the slot and emitted IE code, yet no TPOFF64
appeared: `.rela.dyn` is SIZED by a pre-count (`rela_dyn_glob_count`)
using the old rule, and the scan's extra write landed past the section
end into the not-yet-emitted `.rela.plt` bytes, which then overwrote it
-- silent vanishing, no corruption, no error. Instrumentation proved
set/planner/emission all agreed (`m=true`) before the count was found.
The count now mirrors the scan entry-for-entry with a comment naming
the hazard. Rule: every `.rela.dyn` writer needs its counter updated in
the same commit; grep `rela_dyn_glob_count` when touching the scan.

## Honest scope notes

- The 5.1 difference is STRUCTURAL, not behavioral: the executable's own
  reads resolve to its own definition either way (first in lookup
  order), so LE-vs-IE answers identically at runtime -- measured: bfd
  itself prints 42 under an LD_PRELOAD interposer. The gate
  (`check_tls_pie_preemptible.sh`) therefore pins structure (TPOFF64
  present iff preemptible, GD and GOTTPOFF shapes, three LE controls)
  plus runtime correctness. Mutation proof: fails (no TPOFF64) on the
  pre-fix tree, passes here; the got64 gate passes on both (independent).
- `@object`-in-`.tdata` (hand-asm-only; compilers emit STT_TLS) stays on
  the old path: the house-wide `is_tls` convention
  (`(info&0xf)==STT_TLS`, shared with the `.so` emitter) keys the fix,
  and widening it would change the working is_dynamic path. No
  regression (identical to before); unifying TLS-ness on SHF_TLS section
  membership is a separate, cross-emitter task. Filed as TODO.
- GOTTPOFF against a NON-TLS symbol still takes the old path (garbage
  TP offset, pre-existing, assembler-rejected in practice). TODO: refuse
  with GNU's wording alongside the GD form.

## Validation (this tree, `41d83585`)

- `ci_local.sh --fast`: 100 passed, 0 failed, 5 skipped (98/0/5 at S23;
  +2 = the new gates). rustfmt + clippy green, zero-warning builds.
- Linker suite: 301 pass, 0 fail, 0 warn, 0 skip (300 at S23; +1 = the
  adopted `got64_spelling_matrix`, no skips). Unit tests: 3868/0.
- Gates: got64-old-spelling + tls-pie-preemptible PASS; notype-code +
  copy-alias-sizes re-PASS; both new gates mutation-proven (see above).
- Repro/probe baselines carried: edge/d1/d3 MATCH, d2 known 2.44/2.47
  drift, weak-COPY ours=bfd, mem-mem encoder=GAS (S23, undisturbed --
  no encoder or COPY-path changes this session).

## TODO for future agents

1. (carry) Promote the two gates into `run_linker_tests.py` if a TLS/
   GOT64 suite section wants them; the gates run in CI either way.
2. (new) Unify global TLS-ness on SHF_TLS section membership (see scope
   notes); fixes `@object`-in-`.tdata` in exec AND `.so` emitters.
3. (new) Refuse GOTTPOFF/GD against non-TLS globals with GNU's wording.
4. (carry) `@PLTOFF` of data in `-shared` stays refused (S22 design);
   bfd parity there means staying refused.
5. (dropped) SEC `%rcx`-staging instrumentation: skipped with evidence
   three sessions running (never fires); re-instrument only if reg_cache
   is touched. Weak-UND `OBJECT`-vs-`NOTYPE` tag: cosmetic, zero
   functional impact, deliberately left (S22 dynsym-fidelity work must
   not be risked for it).
