# Red-team review: GNU as reference integrity

This report records the oracle-only stage, **not a generated-code speedup**.
The later [SSE2 runtime follow-up](../sse2-strict-recip-2026-10-25/README.md)
changes the Rust compiler; its measurements and numerical boundaries are
reported separately. The historical measurements below are not attributed to
that runtime change.

The rebase onto the subsequently merged `0F 01 <ModRM>` distiller work retains
its system-instruction rows. With both repairs the regenerated snapshot has
2,195 names/3,975 forms, and all 1,638 encoder rows pass strict checking.
Twenty-three GAS tests now pass, including the upstream ModR/M-group and
coverage-triage and ALU-family regressions. The subsequent coverage-reporting work is retained;
it changes no Rust compiler source or native measurement.
The earlier counts below describe the original oracle-only experiment.

## Decisions on the upstream work

I agree with retaining a lone F2/F3/66 opcode byte and with pinning the original
GNU as table digest. ANDN/BLSI really use the lone-byte shape. I also agree with
an explicit, witnessed SETCC-to-SETC alias rather than treating a reference gap
as successful coverage. Native GAS2.47 independently accepts the motivating
BMI/EVEX instructions.

I disagree with treating the empty uncovered list as exhaustive encoding
verification. Six independent gaps remained beneath it:

1. The evaluator's claimed C precedence was wrong. `1 + 2 << 3` evaluated to17,
   rather than24. Native GCC16.2, Clang23.1.3 and LCCC agree with the corrected
   evaluator for1,024 generated expressions; the previous evaluator disagrees
   on116. This demonstrates the grammar defect, **not**116 corrupt entries in
   the pinned table.
2. `"Vex" in "EVex128"` is true. Substring family inference manufactured VEX
   alternatives for EVEX-only rows. W modifiers are shared by both families
   and cannot themselves prove VEX permission.
3. Blanket VEX/EVEX/legacy equivalence in `_same` then hid family mismatches.
   The repair recognizes exact family attributes and implicit masked EVEX rows,
   instead of solving false findings by accepting every prefix family.
4. Comparing ModR/M digits returned early, skipping W comparison. Both known
   constraints must agree. Raw `VexW=1/2` also needs the enum interpretation from
   GNU `i386-opc.h`: VEXW0=1, VEXW1=2, VEXWIG=3. Unknown W remains unknown.
5. Blindly prepending `v` invented `vandn`, `vblsi`, `vblsr`, `vblsmsk`, `vshrx`,
   among others. GAS naming is derived from the actual SSE2AVX attribute, not
   from the encoding family. Generic removal of `v` during lookup was another
   route around this check and is removed.
6. `--strict` printed uncovered mnemonics but returned success when they were
   its only defect. It now fails for findings, unmodelled rows **or** uncovered
   names; non-strict inspection retains its existing success behavior.

These corrections reduce the same-source snapshot from2,457 names/6,051 forms
to2,134 names/3,913 forms. Fewer entries are not reduced implementation coverage:
all1,637 previously checked encoder rows still pass, with zero findings,
unmodelled rows or uncovered names. The removed synthetic names/forms were not
proof of assembler support.

## Independent evidence and refusals

- The original288,187-byte table was retrieved through GitHub's API. SHA256:
  `a48171c05930cbe120a26fdf802689a4865607b3fe42eca784541dbf7ba81179`.
  The generated snapshot is regenerated, not manually patched.
- Seventeen hermetic tests cover both backends and the new refusal boundaries.
  Running the new test methods against the previous implementation produces
  failing assertions, not collection errors.
- Eleven native GAS2.47 cases include real EVEX FPCLASS/BLEND, BMI ANDN and VEX
  SQRT bytes; forced VEX FPCLASS/BLEND and five invented names are rejected.
- The1,024-expression C oracle uses the existing Godbolt client's local compile
  helper plus native execution, checking stdout and exit status across exact
  GCC16.2, Clang23.1.3 and the warning-denied fastbuild LCCC binary. This is local
  semantic evidence, not a live Compiler Explorer receipt.
- The independent SDM/XED gate still checks2,595 rows,1,436 classes and4,992
  catalogue encodings. Native assembler differentials remain necessary.

The GAS verifier is still a **row-level existence check**: a modelled row may
have multiple operand forms, and at least one must match. It does not prove all
forms, CPU feature legality, ABI, unwinding or runtime semantics. Explicit
string/WRMSRNS limitations and extended-map delegation remain; no assertion of
complete Intel SDM coverage follows from an empty list.

## Toolchain and scope discipline

The cache was enumerated across all available toolchain tags. Rust1.97,
GCC16.2, Clang23.1.3, mold2.42.1 and binutils2.47 payloads were hash-verified.
GAS2.47 was built with two jobs. LCCC/lccc-ld were built with warning-denied
`fastbuild -j2`;8GiB swap was active. `cargo fmt --check` is targeted formatting,
not local CI. Full local CI was not run; hosted checks decide integration.

The prior assertion that the sandbox has no route to Rust is stale: the separate
`rust-cache-1.97.0` tag supplies it. The fact that `harness-cache-1` alone lacks
Rust never established absence from every cache.

Valgrind/Callgrind are available from the verified Debian payload. The cache
provides working32-bit C runtime/development support and a native GCC16 `-m32`
link/run probe passes. Its gcc-multilib meta-packages have missing x32
requirements, so full package configuration is **not** claimed. ICX2026.2,
AArch64 GCC/sysroots, gdb, pahole and objtool are not present in these payloads.
No use of those unavailable tools is invented. They are not substitutes for
parsing and semantic tests of this Python change; architecture and authenticated
ICX checks remain hosted responsibilities. Remote Godbolt/vendor hosts are not
reachable under this sandbox's outbound allowlist.

## Backlog priority

MB-43's trust boundary was repaired before attempting a speculative optimizer
change on top of an overly permissive reference. MB-44's LZ4 match-extension and
nbody gaps deserve fresh output-checked measurements; old ratios are not current
performance evidence. Fresh paired and bounded Callgrind receipts are recorded
separately. No runtime fix for these workloads, no closure of MB-01/02/04/10,
and no universal compiler supremacy is claimed by this patch.

## Fresh MB-44 screening (current-main binary, not a patch speedup)

`runtime.json` / `runtime.md` retain the canonical randomized15-round paired
runner receipts for the two historically worst workloads. GCC16.2,
Clang23.1.3 and LCCC run with matched `-O2` defaults, CPU0 affinity; no samples
or outliers are silently removed. GAS2.47 and mold2.42.1 are first on PATH;
reference-linker provenance was separately verified in ELF `.comment`.

| Workload | LCCC / GCC runtime (95% interval) | LCCC / best available reference |
|---|---:|---:|
|nbody|1.4361 [1.4268,1.4521]|1.4948 [1.4835,1.5229], Clang|
|lz4_match_extend|1.0007 [0.9647,1.0164]|same, GCC|

Minimum arm time is237.9ms; no timed arm is below200ms. The evidence does not
confirm the historical1.937x LZ4 gap. It **does** confirm nbody as the more urgent
of these two investigations. This is a virtualized Xeon host with no PMU, not
Raptor Lake/EPYC hardware. ICX was unavailable locally, so “best” means only
the two available references, not every C compiler.

`callgrind-index.json`, per-case manifests and compressed raw profiles retain
Valgrind3.19 counts with pinned cache geometry, checked native/instrumented
outputs and equal-length aa/bb paths. Both selected cases completed without
failures. Nbody:611,586 versus516,480 Ir (1.1841x). LZ4:1,068,469 versus922,770
Ir (1.1579x). These are reduced sizes under `-march=x86-64-v3 -fwrapv`, **not**
the native runner's defaults. Library/startup costs are included; no inference
from simulated Ir to physical cycles or a runtime speedup is warranted.

The native `.text` bytes of FPCLASS, BLEND, ANDN and SQRT also match between
the built LCCC assembler and GNU as2.47 exactly. The referenced header enum
source `opcodes/i386-opc.h` has SHA256
`875ad755f4f62788b5915d19ecbdbc67b629d17d55f6c7b71ebc81003fca86b6`.
None of the5,223 accepted expressions in the pinned current table changes
value because of the precedence repair (5,339 attempts,116 rejected): that part is a latent grammar fix,
not evidence of currently incorrect encoder bytes.

Reproduction (modern tools first on PATH, baseline source saved separately):

```sh
python3 engineering/evidence/gas-reference-redteam-2026-10-25/native_proof.py \
  --out /path/to/proofs --baseline /path/to/previous/gas_encoding_crossref.py
python3 tests/benchmark/run_benchmarks.py \
  --only lz4_match_extend,nbody --compilers lccc,gcc,clang \
  --gcc gcc-16 --clang clang-23 --reps 15 --strict --json runtime.json
python3 scripts/callgrind_corpus.py --mine target/fastbuild/lccc \
  --ref gcc-16 --gcc-include-command gcc-16 --only lz4_match_extend,nbody \
  --out /path/to/callgrind
```

No local CI was run. Targeted formatting, the strict reference gates, native
proofs and focused oracle self-tests are distinct from final hosted CI.
