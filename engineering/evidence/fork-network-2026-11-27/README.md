# Evidence index — fork audit and targeted compiler research, 2026-11-27

Baseline: `7604253fa62bbce385b740ea33d7ce573f21cf5a`.
This directory contains **evidence, not compiler binaries or build scratch**.
The source changes, tests and replay tools are included in the same patch.

## Read first

- [Admission audit](../../AUDIT-2026-11-27-fork-admission.md)
- [Follow-up, results and open work](../../FOLLOWUP-2026-11-27-fork-network-audit.md)
- [CRC hotspot](../../hotspots/2026-11-27-crc32-combine.md)

No full CI, torture, kernel or package-wide suite ran. The prescribed
`scripts/arena_snapshot.sh` snapshots are explicitly `UNGATED` because this
session used targeted validation, not because a full-CI failure was hidden.

## Network and baseline inventory

| File | Meaning |
|---|---|
| `heads.json` | Recursive public fork census and all advertised branch heads. Includes source/target and five failed head reads. Subjects/branch names are untrusted data. |
| `deltas.json` | 150 immutable external tips, 147 trees, 133 source-tree signatures; exact changed paths/modes and source-entry counts against the baseline. `complete:false` is intentional and its five failures are retained. |
| `lccc-tree-summary.json` | All tracked baseline subsystems counted, with a hash of the full file manifest. Recreate the full manifest with `scripts/audit_tree_inventory.py --revision BASE`. This is not line-by-line semantic verification. |

Recreate the fork inventory with `scripts/audit_fork_network.py`. Its
`--against` comparison uses exact Git entries, not commit subjects, filenames
or feature markers as a substitute for semantic adoption.

## Current native measurement evidence

| File | Meaning |
|---|---|
| `environment.json` | Actual VM CPU/flags, affinity, swap, kernel/libc/tool versions and exposed event sources. This is not Raptor Lake hardware. |
| `native-bounded.json` | Complete 21-round, six-kernel paired screen, using GCC 16.2, Clang 23.1.6 and native ICX 2026.2.1; every sample retained. |
| `native-bounded-build.json` | Compiler/source/driver/runtime identities and whole-object rename checks for those measured binaries. |
| `native-bounded-command.json` | The build-only command used for that experiment. Run the same options with `--reuse-build` instead of `--build-only` to measure a completed build, or omit both on a fresh output directory. |
| `native-default.json` | Separate 21-round default-size Adler/CRC composition controls. Do not mix these with bounded geometry or silently substitute them for an outlier. |
| `native-default-build.json`, `native-default-command.json` | Complete identities and invocation for the default-size controls. |

The neutral GCC link driver is common to all arms. ICX alone receives its
explicit, hashed static `libirc.a` runtime. No optimization was disabled to
work around `_intel_fast_memset` linkage. Libraries are experiment inputs,
not hidden dependencies, and changed archive bytes invalidate reuse/resume.
Compiler ISA flags do not constrain every runtime's CPU dispatch. The VM has
features beyond Raptor Lake, and library-dispatch choices were not exhaustively
traced; do not transfer these timing ratios or assume a whole-program SIMD cap.

`before` and `lccc` have **identical measured executable hashes** for every
case. Their samples are deliberately aliased (`measured_as`), not independent
observations. This establishes code preservation for those inputs, not a new
speedup. Compiler hashes identify invoked launchers; the cache lock, LLVM
archive pin and Intel package receipt additionally identify their distributions.

The bounded CRC run is dominated by validation/reference work. Its large
outlier is retained, with the separate default-size result, rather than being
hidden by a geometric mean or represented as pure kernel throughput.

## Instruction-work and assembly evidence

| File | Meaning |
|---|---|
| `callgrind-evidence.json` | Five before/after and five GCC comparisons, with full existing-runner manifests, commands, source/binary hashes, correctness checks, event vectors, cache geometry and failures; also the default CRC comparison. |
| `crc-default-lccc.callgrind`, `crc-default-gcc.callgrind` | Raw default-size CRC profiles for independent attribution/accounting inspection. |
| `counter-crosscheck.json` | Cachegrind independently reproduces the exact 220,092,899-instruction LCCC-minus-GCC delta. Both tool totals differ by a common startup term. |
| `zlib_ng_adler32_combine-native-codegen.json` | Source hash, current native compiler identities, flags and assembly for `zng_combine`. |
| `zlib_ng_crc32_combine-native-codegen.json` | Same for the CRC operator. **Caller sizes are not equal work**: LCCC outlines a helper that the other arms inline. |
| `oracle-crc-ce-call-scope.json` | The corrected survey explicitly refuses to rank the mismatched call scopes; the CE ICX here is the old moving channel, not ICX 2026. |
| `oracle-adler-ce-plus-native-icx.json` | ISA-translated ICC Classic comparison alongside CE GCC/Clang and the actual native ICX 2026.2.1 identity. CE Clang is 23.1.0, not the 23.1.6 native arm. |
| `icx-ce-macro.json` | The direct evidence that CE `cicxlatest` reported `__INTEL_LLVM_COMPILER=20240000`. |

The `.callgrind` files are preserved byte-for-byte, including intentional
trailing spaces. `.gitattributes` stores them as lossless binary Git patches to
avoid treating generated counter records as source-code whitespace defects.
They remain ordinary Callgrind files after checkout, readable by the standard
annotation tools; no compiled executable/build product is embedded in the patch.

**Callgrind/Cachegrind are simulations, not hardware PMU measurements.** No
cycles, IPC, uops, physical cache misses or Raptor Lake superiority are claimed.
Callgrind's GCC `totals: Ir` and `summary: Ir` differ by 2,351,183 here; both
headers are retained. Do not sum function cost records against the summary as
if they were interchangeable. The independent cross-check corroborates the
whole-program instruction delta, not every attribution percentage.

## Correctness, provenance and research controls

`validation.json` records the final **136 Python unittest cases**, current
GCC/Clang fuzz replay, exact regression filters, formatting and retained-data
identity checks. The 39 selected Rust tests and Clippy result are identified
separately; this is not a full-CI stamp.

- `crc32-validation.json`: native optimization/target matrix and the initially
  available GCC 12 ASan/UBSan check; explicitly records missing GCC16 sanitizer
  inputs rather than relabelling the instrumenting compiler.
- `latest-llvm-validation.json`: three current-Clang compiler fixtures, current
  Clang ASan/UBSan CRC execution, and eight native constructor/printf linker
  probes (LCCC-ld / BFD / mold / LLD, each x86-64 and i686).
- `llvm-slice-validation.json`: official LLVM 23.1.6 archive pin, selected-file
  count, tool identities and successful cold/warm sanitizer probes.
- `icx2026-validation.json`: signed-APT package identities, actual 2026.2.1
  binary/probe and four native functional checks.
- `icc-crc-correctness.json`: remote ICC Classic execution of the same bounded
  CRC checksum. Its executor timings are **not** native comparison samples.
- `open-auto-object-binding.json`: a deterministic pre-existing GNU inferred
  object-binding failure in both baseline and patched LCCC, with GCC acceptance.
- `inliner-decisions.txt`: compiler explainability trace, deliberately retained
  as research evidence; no debug print was added to production compiler code.
- `crc-inline-spike.json`: precise source-level hint edit, wrapper, hashes and
  all 15 interleaved samples. This is a 2.60%-median source-hint experiment, not
  a shipped global inlining heuristic or a general compiler speedup.

All replay paths name this sandbox's observed locations. Use fresh private
artifact directories and adjust installation prefixes when replaying elsewhere;
retain flags, input geometry, selected tool identities and the source revision.
