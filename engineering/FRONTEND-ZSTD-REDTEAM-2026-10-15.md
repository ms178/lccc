# Frontend defects exposed by broad patched-zstd validation

Base: main f07; initial candidate S85. Full library units at S85 passed
4,803 tests (8 ignored). The first format-enabled real-zstd build and shared
GCC-built upstream fuzzer/zstreamtest drivers passed six 500-iteration tests.
The four-arm CLI run executed 3,209 commands and was **not green**.

## Reproduced compiler bugs, not codec-workload special cases

1. **Token boundaries confused with source whitespace.** Expanded output was
   used to decide whether an identifier belonged to an original pp-number.
   `EXPAND(MAJOR.MINOR.PATCH)` became `"1.MINOR. 7"`, not `"1.5.7"`.
   Anti-pasting spaces then polluted stringification. An isolated prototype
   differed from GCC16 on 25/292 probes before repair and 0/292 after repair.
   The repair carries synthetic boundary markers through rescanning, discards
   them for stringification, and serializes them as spaces for final token
   output. Real whitespace and real pp-numbers remain distinct. Both expansion
   and parameter substitution consult their input prefix, not expanded output.
   This is not a blanket removal of the pp-number rule.
2. **Enumerator attributes created spurious enum constants.** With LZ4's obsolete
   enum aliases enabled, `alias __attribute__((deprecated)) = original` was
   parsed as several enumerators, including a new declaration of `original`.
   Both S85 and main produced block-size ID 10 and mode 4 where GCC produced 4
   and 0. A GCC-built interposer proved those wrong values reached
   `LZ4F_compressBegin`, which failed with `ERROR_allocation_failed`. A minimal,
   header-independent native test reproduces the problem. The parser now consumes
   GNU/C23 enumerator attributes before the initializer and isolates their
   attribute state from the enclosing declaration.

Both new native C regression programs return 0 with GCC16 and 1 with both S85
and main-f07. These are genuine red-before cases. New Rust tests cover nested
expansion, source whitespace, real pp-numbers, token pasting, final serialization,
GNU/C23 enum attributes and explicit/implicit/negative enum values. S86 full units subsequently passed 4,806 tests (eight ignored). Fresh native,
cross-target, CE and CLI results are in
[the completed measurement report](MEASUREMENTS-S86-FOLLOWTHROUGH-2026-10-15.md).

## Harness correction and control isolation

`--patch-from` over stdin requires `--stream-size`; the first matrix omitted it
and GCC itself rejected the invocation. The corrected matrix supplies the exact
input size. This was a harness failure, not a compiler regression.

The first run also stopped each LZ4 case on the candidate failure, before testing
main compression. Separate executions confirmed main fails identically. The
final candidate/GCC/system CLI matrix will be kept separate from this known-bad
main-control diagnostic; no control failure will be reclassified as success.
All future matrix records include the CLI ELF hashes.

The repaired compiler gets a **new executable path and fresh CMake directory**.
Overwriting a compiler binary at an unchanged path is insufficient to make Ninja
recompile already-built C objects. The S85 and main artifacts remain unchanged.

Evidence: `engineering/evidence/redteam86-frontend/` and the workspace's
`REDTEAM85-FRONTEND-RESEARCH.tar.gz`. The isolated PP prototype uses standard hash
maps and a flat line resolver; it does not validate line-mapping behavior. Full
compiler regressions and FAST remain necessary. No broad speed win is claimed.

## Frozen replay verification (follow-through)

Run `python3 engineering/evidence/redteam86-frontend/prototype.py --out /tmp/pp-replay`.
The helper now requires a separate output directory and resolves tools through
PATH. Its preprocessor utility dependency is frozen alongside `before.rs`;
no live source checkout or `/home/user/lccc` absolute path is needed. A fresh
execution reproduced exactly 25/292 pre-fix mismatches and 0/292 prototype
mismatches against GCC16. Both conditions are asserted. These remain isolated
prototype tests, not a replacement for the production compiler's regression gate.
