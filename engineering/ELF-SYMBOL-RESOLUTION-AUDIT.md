# ELF symbol resolution: modes, provenance, and validation

This is the continuation of the [backlog ROI audit](BACKLOG-ROI-2026-11-18.md)
for [PR #1200](https://github.com/ms178/lccc/pull/1200). The relevant
backlog areas are MB-02 (option trust), MB-13 (i386 link parity), and LK-06/07
(ELF symbol resolution and input discard). The controls are the exact
rebased pre-change fastbuild and GNU ld 2.47; the committed regression suite
also supports the GNU linkers installed by CI. Comparisons are bounded to
these fixtures and do not establish general GNU linker equivalence.

## Independent review items

Rows refer to production defects or costs, not to the number of input orders,
architectures or test cases that cover them. Related architectures share a row
unless they maintain separate algorithms and output contracts.

| # | Production change | Witness or check |
|---|---|---|
| 1 | Reject duplicate strong definitions in the script emitter before GC/layout. The separate ELF32 loader had no conflicting-symbol check. | `script-duplicate`, `script-duplicate-gc`, `script-archive-duplicate`; `/DISCARD/` is not a duplicate-definition waiver. |
| 2 | Reject duplicate strong definitions in the RISC-V resolver for both output kinds. | `riscv64/{exe,so}-duplicate` and distinct ABS; no output on failure. |
| 3 | Implement the actual first-strong-wins opt-in in native, `-T`, and `-r` paths instead of warning or accepting it without effect. | `*-permit-first-absolute`, `script-opt-in`, `r-muldefs`; `--fatal-warnings` must not penalize a supported spelling. |
| 4 | File-mode loading tracks consumption per operand, not per filename. A repeated object is a second input, not an archive rescan. | `script-repeat-operand`, `r-repeat-operand`; unchanged thin/regular archive tests. |
| 5 | ELF64 registration admits equal true `SHN_ABS` definitions, without treating identical section-relative offsets as equal absolutes. | `*-equal-absolute` against `*-different-absolute` and ordinary duplicates. |
| 6 | GNU UNIQUE participates in strong registration and `-r` selection. It does not bypass the duplicate check. | `exe-unique-duplicate`, `r-unique-duplicate`, same COMDAT signature control. |
| 7 | Shared ELF64 registration ranks COMMON ahead of WEAK and behind strong definitions. | Both orders in executable and DSO; native runtime checks of a zero-initialized COMMON and a deliberately nonzero weak initializer. |
| 8 | ELF64 tentative definitions merge alignment and size independently in the registering table. | `so-common-max-*` in both orders; BFD, not a distro mold assumption, specifies the independent maxima. |
| 9 | ELF64 COMMON allocation validates power-of-two alignment and checks the aligned address and end arithmetic. | `common_allocation_tests`; no large buffers or allocation are needed to exercise the overflow branches. |
| 10 | A discarded ELF64 COMDAT definition cannot enter registration even when its name was never seen. | `*-losing-unique-name`; a same-name live winner still resolves. Default-version aliases also respect membership. |
| 11 | ELF64 DSO layout skips losing COMDAT bytes and prunes associated FDEs, before ICF. | `so-comdat-size`, distinct ghost name, existing EH/ICF suite. |
| 12 | Script placement drops losing COMDAT sections, and script GC cannot use their copies as roots. | `script-comdat` after direct and archive controls; existing script KEEP and FDE tests. |
| 13 | A weak reference to a discarded script definition no longer becomes a genuine unresolved weak reference. | `script-weak-discard` fails; `script-weak-without-definition` still links. |
| 14 | The script dynamic symbol filter sees hidden references as well as hidden definitions. | `script-dso-hidden-reference`, existing versioned vDSO cases. |
| 15 | Script DSO export planning excludes explicit discarded definitions. | `script-dso-discard-export`; intentional LCCC policy: BFD can expose a zero-address `ABS` name even though the section is absent. |
| 16 | Script input matching excludes `SHF_EXCLUDE` sections, including allocated ones in a final link. | `script-shf-exclude`; a normal placed section still emits. |
| 17 | `-r` serializes the independently merged COMMON size/alignment and never turns a real COMMON into a weak section definition. | `r-common-max-*`, `r-common-over-weak`; `st_value` remains alignment, not an output address. |
| 18 | `-r` preserves a later required undefined reference over an earlier weak-only reference. | `r-upgrade-strong-reference`; the output still carries the remapped relocations. |
| 19 | `-r` folds visibility across its inputs even when symbol selection does not replace the row. | `r-hidden-reference`; preserves non-visibility bits of the chosen row. |
| 20 | `-r` does not manufacture an undefined name for a losing, unreferenced COMDAT definition. | `r-losing-unique-name`; only retained relocation sites justify emitting an unresolved row. |
| 21 | `-r` validates shared options and does not silently ignore a malformed `muldefs` spelling. | `r-invalid-muldefs` under `--fatal-warnings`. |
| 22 | RISC-V selects tentative definitions before allocation; a surviving COMMON receives both maxima and a superseded one receives no BSS slot. | `riscv64/*-common-max-*`, `*-strong-over-common`, `*-no-dead-common-allocation`; check `sh_size`, not just the winning name. |
| 23 | RISC-V layout drops losing COMDAT members before relaxation, while keeping object/section indices stable. | `riscv64/*-comdat-*`; live allocated references to discarded locals cannot obtain a made-up address. |
| 24 | Preserve visibility from discarded COMDAT entries separately from reachability and winning section identity. | `*-comdat-merged-visibility`: hidden on a kept static or partial definition; absent from shared dynamic exports. |
| 25 | Reject live local relocations into retired COMDAT members on ELF64 final and partial links. | `*-live-local-in-loser`; dead GC sources and edited unwind/debug metadata are not program demand. |
| 26 | Model a live global reference to a retired copy as demand, so an executable cannot silently resolve it to zero or the start of an unrelated section. | `*-live-global-in-loser`; shared links retain an undefined import + relocation rather than a `RELATIVE+0` placeholder, a documented LCCC policy where BFD differs. |
| 27 | AArch64 executables omit COMDAT losers in layout and GC, not just in registration. | `aarch64/exe-comdat-size` and the same-name survivor and local-reference controls. |
| 28 | x86-64 executable COMMON symbols report the real `.bss` index. | `exe-common-max-*` asserts `st_size`, aligned final address, NOBITS section index and exact allocation size. |

Two link-work reductions accompany the correctness changes: ELF64 registration
no longer reserves hash entries for every local symbol, and a COMDAT object's
losing section indices are collected once instead of repeatedly searching
its group table for each conflicting symbol. RISC-V updates winners in place
instead of allocating an owned key for every occupied lookup. These are
linker-time changes, not claims about generated application cycles.

## Design and safety review

- **`ObjectSet`, `discarded_members`, `plan_comdat_iter`:** the claim owner is
  the input list in link order. Group signatures and member indices are read
  at the object boundary. Retain first occurrence; tolerate out-of-range group
  members without indexing them, and saturate the non-semantic byte counter.
  Empty group sets allocate nothing. Re-walking prior objects is quadratic;
  checking only collisions misses names first introduced by a discarded copy.
- **`register_symbols_elf64`, `merge_common`, `replace_keeping_visibility`:**
  only regular strong/UNIQUE definitions can override a real definition by
  binding rank. Dynamic/shared imports keep their existing separate policy.
  A COMMON entry's value is an alignment until layout; neither field chooses
  the other field's maximum. Visibility is restrictive, not a winner-only
  property. Report both strong origins without copying filenames into rows.
  Reservation based on the local prefix is only a capacity hint, never a
  bound on semantic scanning. No shared mutable/synchronized state is added.
- **`allocate_common_symbols_elf64`:** derive padding from checked power-of-two
  alignment, then check the size addition. This deliberately refuses an
  invalid description rather than wrapping into another allocation. Failures
  drop the local output plan and do not emit an image. The four real callers
  propagate the error.
- **`load_inputs_into`, `lccc_ld::run`:** ordered, per-slot consumption makes
  group scans and deliberate repeats independent. The exact `muldefs` token
  reaches both loading and output selection. Modes that implement it no
  longer produce a false warning; a misspelled value is not an opt-in.
- **`link_with_script_machine`:** separate COMDAT eligibility, symbol
  selection, reachability, and placement. Do not promote a losing weak
  definition after discarding a stronger one. GNU `SHF_GNU_RETAIN` reachability
  is preserved even if the retaining input is omitted by `/DISCARD/`.
  Keep discard provenance for ordinary relocations and DWARF; only the
  `--defsym` expression planner receives the historical temporary absolute
  view of orphan values. Export selection happens before synthetic dynamic
  layout, not as a late mutation of `.dynsym` indices.
- **`link_relocatable`:** output row numbers remain stable through strength
  upgrades, so prior relocation remaps stay valid. The borrowed-name map is
  bounded by the input objects' lifetime; output rows still own their names.
  COMMON values are never rebased. No relocation is applied at the partial
  link; stack validation and DWARF offsets were checked on the result.
- **RISC-V `build_global_symbols` and `discard_comdat`:** pending COMMON
  is a non-index sentinel consumed before any output section lookup. Growth
  is checked before converting to host `usize` or reserving a zero-filled
  vector. Relocations and symbols retain original indices; retired sections
  are absent from layout/probes, and relevant FDEs are pruned first.
  No new register assignments, codegen transforms, pointer casts, locks,
  or atomics were added by the linker continuation.

## Reproduce

```sh
source engineering/verification/agent-env.sh
export LCCC_SWAPFILE=/var/swap.lccc LCCC_SWAP_SIZE=4G
bash scripts/build_lccc_fast.sh
python3 tests/linker/elf_symbol_contract.py --ld target/fastbuild/lccc-ld \
  --target x86_64 --target i386 --target aarch64 --target riscv64 \
  --json artifacts/snapshots/elf-symbol-contract.json
python3 tests/linker/elf_symbol_contract.py --ld target/fastbuild/lccc-ld \
  --vendor-cc gcc=gcc-16 --vendor-cc clang=clang-23
```

The cross test requires a GNU cross linker per target. For input assembly it
can use a GNU cross assembler or Clang's integrated assembler. Local cross
BFDs in this audit were built from the verified binutils 2.47 source.
The authenticated ICX lane runs the same native runtime matrix with the Intel
PPA version, not a Clang-named substitute. Raw receipts and exact revision
status belong with the delivered checkpoint, not with historical baselines.

## Remaining questions

This does not implement all `-T` COMMON placement or filename-qualified
`/DISCARD/` patterns, add ELF32 `-r`/PIE, or change the documented mold/GNU
COMMON differences. It does not make relocation copying for unrelated
backends into a general-purpose partial linker. Archive demand before
RISC-V COMDAT selection and preserving group metadata across multiple
partial links remain separate work. The compiler peephole share target,
physical-host runtime calibration, and liveness memo feasibility are not
resolved by these linker fixes.
