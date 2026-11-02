# PR831 oracle and acceptance follow-through — 2026-10-15

## Scope and saved source

Upstream was fetched again: main remains `63e783b15586eca86f306bd75a52b82c57304bbb`,
PR831 remains `acd48b0ca8b8427e95d30ecdae72bfb1fd888f38`; rebase reports up to date.
S70–S75 saved each validated repair immediately, without claiming a green gate.
The final complete FAST verdict belongs to the matching-tree delivery ledger;
interrupted runs and individual gate passes do not constitute acceptance.
No full/slow local CI was requested or used for this follow-through.

## Repairs found by adversarial review

- Removed one **byte-identical duplicate unit-test block**, retaining its assertions.
- `oracle_asm.py` now fails when any requested compiler/function is unavailable,
  clears stale requested assembly outputs, validates output labels, and accepts
  single dash-prefixed flag strings correctly.
- `tune_oracle.py` uses private temporary directories, preserves quoted flags,
  validates remote replies, recognizes commented/quoted labels, rejects empty
  selections/partial comparisons, and cannot leave a prior success status after
  a local compile abort. Its matrix command rejects missing requested tunes.
- The two local oracle compile helpers now share one private-directory path:
  failure/timeout cleanup, bounded subprocess time, and missing/empty assembly
  rejection. A PID-based filename alone did not protect concurrent threads or
  pre-existing symlinks. A pre-created empty output was not proof of compilation.
- `godbolt.py compile --execute` no longer returns success for a nonzero,
  missing, timed-out, or explicitly unexecuted program result.
- CFI parity no longer silently omits a failed/no-output no-unwind twin after
  baseline compilation succeeds. Failures preserve their diagnostic. Its scratch
  directory is private, avoiding stale PID-reused output contamination.
- Thirteen offline oracle tests and three fake-compiler CFI tests exercise these
  failures. They live under the **actual FAST corpus discovery path**. The 14
  existing codegen metric contracts are also wired into that discovery path.

These are harness/correctness repairs, **not runtime optimizations**. No backend
profitability guard, wrapping rule, ABI assertion, or performance threshold was
weakened to obtain a favorable result.

## Current local evidence

- All-target Rust units: **4,782 pass, zero fail, eight ignored** (4,781 library
  tests plus one linker-binary test). Default and all-feature/all-target checks pass.
- `fastbuild`, optimization level 1, exactly two Cargo jobs, debug info and
  incremental compilation disabled; six GiB swap active on this two-GiB VM.
- Rebuilt compiler SHA256:
  `bca00af65791e99ac8f1cbc56ce6a5d49a99965540810715314cd8846c76be49`.
  The earlier measured compiler is `3b78f2e710e8c0d7fd3949a005623d3abcbe3455fbba085caeb44df2b9f40656`.
  They are **not the same compiler binary**. Ninety exact-command comparisons
  prove their complete generated benchmark ELFs identical: 45 native/default
  and 45 bounded/Callgrind cases. This is continuity, not another timing run.
- All seven fresh Haswell assembly listings also match byte-for-byte through the
  repaired helpers and rebuilt compiler.
- Fresh Memcheck: **24/24** wrapping-domain cases across LCCC, Debian GCC16.2,
  Clang23.1.3 and both x86 ABIs; matching expected output, no memory errors.

## Fresh Compiler Explorer requests

A newly created client cache, refreshed compiler inventory, and successful
four-vendor Haswell canary were used. This does **not** claim control over CE's
server-side caches. Requested common flags: `-O2 -march=haswell -fwrapv`.
The existing vendor mapper uses ICC's equivalent `-xCORE-AVX2`; effective flags
are preserved, not silently hidden. Local GCC16 headers are recorded.

**Whole-translation-unit static instruction counts — not speed, dynamic work,
spill causality, or proof that differently vectorized loops do equal work:**

| Case | LCCC | GCC16.2 | Clang23.1.0 | ICC2021.10 | ICX alias |
|---|---:|---:|---:|---:|---:|
| byte_load_reassembly | 153 | 131 | 202 | 218 | 63 |
| expat_xml_scan | 219 | 172 | 222 | 322 | 327 |
| fannkuch | 146 | 142 | 227 | 229 | 268 |
| linux_find_bit_scaled | 148 | 144 | 118 | 187 | 121 |
| lz4_compress | 350 | 295 | 421 | 442 | 363 |
| sieve | 55 | 51 | 345 | 192 | 376 |
| zstd_count | 167 | 91 | 121 | 106 | 152 |

Executed version probes identify GCC16.2.0, Clang23.1.0, ICC macro2021, and
**ICX2024.0.0 / macro20240000**. The refreshed inventory offers no newer numbered
ICX than 2024.0.0; the `latest` alias is not evidence of vendor-latest 2026.
ICC metadata says2021.10.0, with deployment path intel-cpp-2023.2.1.8. Its
10441 deprecation remark remains in the evidence, rather than being suppressed.

| Executed witness | GCC | Clang | ICC | ICX |
|---|---:|---:|---:|---:|
| signed rotation, `-O2 -fwrapv` | exit0 | exit0 | **exit1** | exit0 |
| four wrapping domains, expected 28 / -1073665362 / 1 / 10 | match | match | **third output4** | match |
| strict FENV | exit0 | exit0 | **exit41** | exit0 |

FENV uses `-frounding-math -ffp-contract=off` for GCC/Clang and `-fp-model strict`
for ICC/ICX. GCC's earlier exit52 used different flags; it is not contradicted
or erased by this strict-profile pass. ICC repeats BOTH integer failures with
`-fp-model strict`; its `-O0 -fwrapv` controls pass. The wrapping program exits0
even on ICC's wrong printed result: **exit status alone is insufficient**.
No four-vendor unanimity or universal compiler superiority is claimed.

## Performance disposition and concrete remaining work

Performance approval remains **withheld**. The 20-case latest-main native
GM is1.009473 and AM1.011519; 45-case Callgrind Ir GM1.001028 and AM1.001404.
Fannkuch is about25% slower; 31-pair sieve, XML, and scaled LZ4 follow-ups are
+4.83%, +4.51%, and +2.68%. Byte reassembly improves about10.32% in the nine-pair
sample. Exact intervals, workload definitions and caveats remain in
`PR831-COMPARISON.md`; these VM results are not physical EPYC/Raptor Lake data.

A concrete Zstd opportunity is the adjacent `movl %eax, 12(%rsp)` /
`movzbl 12(%rsp), %esi` relay. The global forwarding handler supports equal
widths and Q→L, not arbitrary narrow extending loads. Generalizing it requires
volatile/alias/overlap/register-view and upper-write proofs plus negative tests;
this observation is **not** an implemented optimization or established cause
of the entire runtime regression. Do not remove the 8→11 ratchet disclosure.

The legacy tight-loop census still needs matched local/remote target flags,
comment-aware labels, maximum unconditional alignment handling, and explicit
failure/coverage reporting before it can support comparisons. Density/rank
surveys remain heuristic screens, never acceptance gates. This review does not
claim every helper or every repository function has been exhaustively audited.

## Reproduction / raw evidence

`engineering/evidence/pr831-review/oracle-recheck.tar.xz` preserves the refreshed
inventory, audit, profile canary, seven complete tables/listings, 20 execution
replies and source probes, quoted commands, local Memcheck report, and Rust logs.
Its SHA256 is in `oracle-recheck-manifest.json`. Exact 90-ELF commands/hashes are
in `post-unit-repair-binary-equivalence.json`. Final FAST logs and acceptance
receipt are published separately with the cumulative patch so a pending run is
never represented as green in this pre-run source report.
