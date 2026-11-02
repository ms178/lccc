# TASK-LK-24 — externally-compiled PIE segfaults at startup (lccc-glibc)

IDs: LK-24 · Priority: **P0** · Base: f657de55 · Hard correctness gate for
the whole glibc runtime campaign.

## Objective

Binaries compiled by external compilers (GCC) and linked/run against the
lccc-built glibc die with SIGSEGV (139) in ld.so before `main`. glibc's OWN
utilities (utmpdump/sprof) run fine — the difference is in what our
ld.so/crt path does for foreign PIE inputs.

Already fixed on main (do not redo): ld.so ET_DYN `e_entry` hardcoded 0,
verdef `vd_next` chain, segment-prefixed store clobbering the pointer
operand's home (`memory.rs:2993`, cited LK-24), versym for plain defined
symbols (`afa22485`), ld.so self-relocation NULL deref
(`linker_common/symbols.rs:132`), `emit_shared.rs:238`.

## Files

`src/backend/linker_common/` (dynamic section, reloc application order),
`src/backend/x86/linker/`, TLS-init triage under `LD_DEBUG=all`,
`_r_debug`/`_rtld_global` integrity.

## Acceptance

- A GCC-compiled `hello` PIE runs against the lccc-built ld.so + libc.so
  (exit 0, correct output) — the smoke gate that currently SIGSEGVs.
- `LD_DEBUG=all` trace clean to the program entry.

## Validation battery

staged-loader smoke (do NOT weaken it) · glibc's own binaries still pass ·
`utmpdump`/`sprof` regression · extern-PIE matrix: GCC/lccc binary ×
glibc/lccc libc · readelf/objdump parity of the dynamic segment vs GNU ld
for the same input.

## Do not

- Do not weaken or skip the smoke gate to make progress (recorded
  standing order from sessions 63/64).
- Do not paper over with lazy-binding changes before the reloc-order diff
  vs GNU ld is understood.
