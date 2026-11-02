# Shared one-hot mask and BitTest width audit — 2026-11-01

Baseline: `0803bb75` (merged PR #1061).  This audit deliberately treats that
merge as a baseline, not as a proof of optimality.

## Reproduced defects / missed optimizations

1. **Shared one-hot masks blocked x86 bit selection.**
   The single-use matcher refused a mask with several readers.  That is the
   right default when a rewrite would leave the mask live, but it missed the
   all-bit-family group where every reader can consume the index directly.

   Probe (`shared_probe.c`, `-O2`, x86-64):

   | compiler | bit instructions in probe | `shared_four` instructions |
   |---|---:|---:|
   | LCCC baseline | 0 | 17 |
   | GCC 16.2 | 0 | 17 |
   | Clang 23.1.4 | 11 | 11 |
   | LCCC after | 11 | 13 |

   Host runtime microbenchmark (`shared_bench.c`, `-O3`, best of 7, pinned to
   one Xeon @ 2.60GHz core):

   | build | seconds | checksum |
   |---|---:|---:|
   | LCCC baseline | 0.147333429 | 34480000 |
   | LCCC after | 0.134144084 | 34480000 |
   | GCC 16.2 | 0.148580310 | 34480000 |
   | Clang 23.1.4 | 0.116334781 | 34480000 |

   The new group pass is **8.95% faster than the LCCC baseline** and **9.72%
   faster than GCC 16.2** on this probe.  Clang remains faster on this specific
   microbenchmark; the audit does not claim a universal win.  The remaining gap
   is register allocation / clear-test flag fusion, not mask materialization.

2. **Narrow `BitTest` used the container width on every backend.**
   `BitSet`/`BitClear`/`BitToggle` had been made width-aware, but `BitTest`
   still reduced indices modulo 32/64 on x86/i686/AArch64/RISC-V.  For an
   8/16-bit IR `BitTest`, an index of 8/16 could select a bit outside the IR
   value.  The fix makes `BitTest` use the same modulo-W contract as the modify
   operations and adds backend unit tests for x86-64, i686, AArch64 and RISC-V.

## Design decision: all-or-nothing, target-aware group rewrite

Alternatives considered:

- **Remove the single-use guard.** Rejected: on AArch64/RISC-V each bit-modify
  lowering rebuilds a one-hot mask, so duplicating readers would duplicate the
  mask.  On x86, rewriting only some readers can leave the mask live and replace
  a cheap logic op with a bit instruction without a net win.
- **Backend-only peephole.** Rejected for this step: the IR already has the
  canonical bit operations, and a group proof at IR level keeps the kill switch,
  differential gates and cross-block use analysis in one place.
- **Chosen:** a pre-pass that proves *every* reader of a one-hot mask is in the
  bit family, closing over copies, complements and test-ands.  It rewrites the
  whole group atomically and only on x86/i686, where the native instructions
  consume the index directly.  One escaping store/call/phi/inline-asm/non-bit
  reader refuses the group.

The pass is split into small functions so the audit census does not grow:
`fold_shared_bit_select`, `analyze_shared_candidate`, `forward_shared_index`,
`shared_group_operand`.  It preserves instruction positions and source spans.

## Tests added / extended

- `src/passes/bit_idioms_bit_select_tests.rs`: shared group, escaping mask,
  shared complement, copy aliases, cross-block readers, AArch64 refusal,
  i686 64-bit refusal, non-bit reader refusal, duplicate operand refusal.
- Backend `BitTest` width tests for x86-64, i686, AArch64, RISC-V.
- `tests/regression/check_bit_select_fold.sh`: shared assembly shapes,
  kill-switch ownership, escaping-mask control, host runtime checksum, pinned
  GCC/Clang oracle defaults and optional ICX oracle.
- `tests/regression/fixtures/bit_width/runtime.c`: shared-mask checksum for the
  mandatory AArch64/RISC-V QEMU gate.

## Oracle receipts

- GCC 16.2.0 and Clang 23.1.4 restored from the hash-verified
  `harness-cache-7` tag and used for the checksums/benchmarks above.
- Shared-mask runtime checksum: `15164872185446587354` (GCC == Clang).
- Local ICX 2026.2 provisioning was attempted with
  `scripts/ensure_icx_2026.sh`; Intel PPA TLS failed with EOF in this sandbox.
  The gate accepts `ICX=<path>` so the authenticated hosted ICX lane / a local
  Intel prefix can run the same checksum without changing the test.

## Census

`python3 scripts/audit_census.py --check` passes with no metric growth:
panic sites remain 877, and the new pass did not increase the >300/>1000 line
function ratchets.
