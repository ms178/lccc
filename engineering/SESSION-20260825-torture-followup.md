# LCCC session follow-up — 2026-08-25

## Baseline and environment

- Rebased checkout: `06ff0fa` (`origin/main` shallow checkout).
- Installed and activated `/swapfile` (8 GiB; `/proc/swaps` reports it active).
- Restored Rust/Cargo 1.98.0 in the persisted workspace.
- Built LCCC with `scripts/build_lccc_fast.sh`, `fastbuild`, `-j2`, and `-O1` as required.
- GCC torture input was obtained externally in `/home/user/gcc-testsuite`; it is intentionally not copied into the repository.

## Evidence collected

The x86 execute harness ran 450 sources at `-O0`, `-O1`, and `-O2` (1,350 cases). 1,331 passed. The remaining cases were recorded in `/home/user/torture-smoke.log` and `/home/user/torture-smoke.json`.

Several link failures are intentional GCC torture negative/link-error tests and should not be counted as execution failures. The remaining failures exposed narrow x86 register-width emission and pre-existing test-only IR initializer drift.

## Changes landed

1. Narrow integer load/store emission now selects `al`/`ax`/`eax` instead of incorrectly spelling `%rax` for byte/word instructions, including direct-register, indexed, and accumulator fallback paths.
2. `load_dest_reg` now reflects the actual narrow instruction destination for signed and unsigned byte/word values.
3. Added the newly required `function_alignments` field to all GVN unit-test `IrModule` literals.

The fastbuild compiler rebuilt successfully after these changes. The canonical `/home/user/ms178-1.patch` and `/home/user/artifacts/` snapshot were refreshed as `S01-torture-triage`; the patch was verified with `git apply --check` against the recorded base.

## Remaining work (highest priority)

- Re-run the focused torture failures after the next build and inspect any remaining `%rax` narrow forms. The current reproducer still shows malformed assembly in an additional alloca/copy path, so the width audit must continue before claiming zero failures.
- Add a regression test for every narrow load/store path, including `I8`, `U8`, `I16`, and `U16`, with register-resident and stack-resident destinations.
- Repair the test linker environment (`/usr/bin/x86_64-linux-gnu-ld` is absent in this VM) or invoke the established standalone `lccc-ld` harness path; do not classify negative link tests as compiler failures.
- Run the full torture corpus at `-O0/-O1/-O2` once the focused set is clean, then run zlib-ng, gzip, and expat functional tests.
- Only after correctness is clean, collect repeated wall-clock/assembly comparisons and use `scripts/codegen_oracle.py` and `scripts/codegen_scoreboard.py`; this session made no unsupported performance claim.

## Guardrails

Every validated change must be snapshotted immediately. Do not include generated `target/`, temporary logs, GCC input, or VM-only artifacts in the patch.
