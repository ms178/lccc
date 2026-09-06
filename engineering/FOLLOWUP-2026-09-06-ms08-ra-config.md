# MS-08 follow-up — invocation-owned RA configuration

**Date:** 2026-09-06

**Implementation base:** `a0090a881d5f799b67e23f53889ab3e9ac0e1e5a` (rebased history contains current upstream `ms178/lccc` `origin/main` `aae69e2b`)

**Scope:** preserve every audited register-allocation/backend `CCC_*` compatibility knob while moving its parsing out of hot per-function paths.

## Decision

**Landed:** an immutable, invocation-owned `RaConfig` is now the sole parser and owner for the audited RA-related policy surface.

`Driver::compile_to_assembly` captures one `Arc<RaConfig>` before the pass pipeline starts. The exact same object is then threaded through:

- `run_passes`, including the pre-pass and inline-phase `CCC_DUMP_IR` checks;
- `CodegenOptions` and `CodegenState`;
- all four backend constructors (x86-64, i686, AArch64, and RISC-V);
- stack-layout/RA setup, live-range construction, linear scans, fusion tables, and parameter-home analysis;
- x86 MachInst selection and the phase-4 peephole safety gate; and
- backend IR dump and allocator diagnostic paths.

The environment remains the intentionally backwards-compatible *construction* surface. No default changes, alias removals, or code-quality policy changes are bundled with this refactor. `LCCC_DUMP_IR` remains deliberately separate driver-level behavior; it was not conflated with `CCC_DUMP_IR`.

There are **73 distinct literal inputs** in the centralized parser: 56 presence switches, 15 textual inputs, and three numeric inputs, with `CCC_TRACE_ALLOCSTATS` intentionally serving both a presence flag and a text filter. Each field declaration in `src/backend/regalloc.rs` records the name, polarity, and default. The old `CCC_NO_*` names remain literal parser inputs rather than being translated or renamed.

Standalone/unit-test compatibility helpers use `RaConfig::default()` and direct standalone backend constructors capture their own configuration. That avoids process-global `OnceLock` state in tests. The production driver path is the one that matters for the one-object-per-invocation contract and always passes its captured `Arc` explicitly.

## Why this is behavior-preserving

This is deliberately a plumbing change, not an allocator tuning change. The only semantic operation at capture time is the historical one for each variable:

- presence switches use `var_os(...).is_some()`;
- textual filters use UTF-8 `var(...).ok()` exactly as before;
- `CCC_LOOP_PIN`, `CCC_HOT_WEB_STEAL`, `CCC_MI_MAX_LOOP_INSTS`, `CCC_EVICT_MODE`, and `CCC_PGO_WEIGHT_MAX` retain their former fallback and clamp behavior; and
- negative flags retain their false/default-off polarity.

The shared `Arc` ensures a single compilation cannot observe conflicting settings in a pass, allocator phase, prologue, emitter, or final peephole pass. It also removes the `live_range.rs` process-global cache behavior that previously made parameterized tests dependent on whichever environment was observed first.

The one plumbing defect encountered during the migration was caught by `cargo check`: the `CCC_DUMP_IR` condition belongs inside `run_inline_phase`, so `run_inline_phase` now explicitly receives `&RaConfig` from its sole `run_passes` caller. No fresh environment read was reintroduced to paper over that error.

## Programmatic configuration coverage

`backend::regalloc::ra_config_tests` uses injected presence/text sources rather than mutating the process environment.

| Test | Coverage |
|---|---|
| `parser_covers_every_boolean_ra_switch_without_process_environment` | Every presence switch is asserted false by default and true when present, including every `CCC_NO_*` switch and diagnostics. |
| `parser_preserves_numeric_text_and_polarity_contracts` | Defaults, valid values, malformed fallbacks, PGO clamp, all text filters/lists, and the scalar-FP enable-over-disable precedence. |
| `explicit_config_controls_linear_scan_without_process_environment` | A caller-supplied `CCC_NO_SEGMENT_SCAN` configuration changes segment-scan behavior without consulting global process state. |
| `parser_instances_are_isolated` | A configured instance cannot poison a subsequent default instance. |

The final focused run reports 4/4 passing. The final complete fastbuild library run reports **1,992 passed, 0 failed, 6 ignored** (1,998 total tests).

A source-level ownership audit dynamically collected the parser literals and searched Rust sources for `std::env::{var,var_os}`, `env_on`, and `env_flag_set` uses of those names outside `RaConfig::from_sources`. It found **73 parser-owned names and 0 offenders**:

```text
PASS: all parser-owned literal CCC_/LCCC_ reads are centralized in
RaConfig::from_sources.
```

The audit result is retained at `/home/user/artifacts/ms08/ra-config-ownership-audit-final.txt`.

## Generated-code and diagnostics evidence

### Default policy identity

The new `scripts/check_codegen_refactor_identity.py` is a strict before/after assembly gate. It scrubs inherited `CCC_*`/`LCCC_*` state, respects adjacent `.flags` files, retains raw assembly/hashes, and only canonicalizes the documented process-ID spelling embedded by `-fprofile-generate`. The canonicalization is narrowly restricted to that flag and those exact generated forms; ordinary numeric or textual codegen differences remain failures.

Against the pristine compiler at `/home/user/.cache/ms08-base-build/lccc-baseline` and the final fastbuild candidate:

| Corpus | Exact assembly | Allowed PGO PID-only equivalent | Real differences | Compile failures |
|---|---:|---:|---:|---:|
| `tests/benchmark/kernel_corpus` | 16 | 0 | 0 | 0 |
| `tests/regression` | 636 | 4 | 0 | 0 |

The four non-byte-identical regression cases are all `-fprofile-generate` cases. Their raw artifacts remain present; only LCCC's generated profile helper/path process-ID suffix differs.

### Every parsed input compatibility smoke

The identity tool now has an explicit repeatable `--env KEY=VALUE` option. It applies a documented bisection setting *after* the default-policy scrub, so a non-default compatibility run does not accidentally inherit unrelated knobs.

A generated final matrix enabled every one of the 73 parser inputs, one at a time, and compared the baseline and candidate on all 16 kernel sources. Every matrix member had `IDENTICAL=16`, `DIFFERENT=0`, and `COMPILE_FAIL=0`: **1,168 successful baseline/candidate assembly comparisons, 0 differences**. The summary and every individual raw comparison are retained under `/home/user/artifacts/ms08/switch-identity-final/`.

Critical x86/i686 paths also received explicit baseline/candidate checks under their relevant settings:

| Path | Configuration | Result |
|---|---|---|
| x86-64 div/rem pairing | `CCC_NO_IR_DIVREM=1` | byte-identical baseline/candidate assembly |
| x86-64 folded-index path | `CCC_NO_FOLDED_INDEX_LIVENESS=1` | byte-identical baseline/candidate assembly |
| x86 phase-4 safety condition | `CCC_NO_MACHINST=1 CCC_PEEPHOLE_PHASE4=1` | byte-identical baseline/candidate assembly |
| i686 mul-accumulate pairing | `CCC_NO_MULACC=1` | byte-identical baseline/candidate assembly |
| i686 folded-index path | `CCC_NO_FOLDED_INDEX_LIVENESS=1` | byte-identical baseline/candidate assembly |
| i686 div/rem pairing | `CCC_NO_IR_DIVREM=1` | byte-identical baseline/candidate assembly |

`CCC_DUMP_IR=1 CCC_DUMP_IR_FUNC=adler8`, legacy `LCCC_DBG_RA=1`, and `CCC_MI_DEBUG=1` diagnostics were each byte-compared between baseline and candidate as well. The filtered IR dump is 140,503 bytes / 4,240 lines on each side, so the migration verifies diagnostic behavior rather than merely successful compilation.

### Allocation-statistics compatibility

The required process-level allocation-statistics smoke is exact under both matching and nonmatching filters. With `CCC_TRACE_ALLOCSTATS=pressure`, both compilers emit exactly:

```text
[RA-STATS] fn=pressure eligible=22 scan=22 assigned=22 spilled=0 segments=32 holes=0 callee-homes=11 caller-homes=13
```

The assembly and stderr SHA-256 values match. With a nonmatching filter, both stderr streams are empty and assembly remains byte-identical. Artifacts and hashes are in `/home/user/artifacts/ms08/trace-allocstats/summary-final.txt`.

## Kill-switch behavior is live, not just parsed

The parser unit matrix proves polarity for every switch. Separately, final candidate-only process smokes prove that representative policy switches actually reach their documented consumers rather than becoming dead fields.

| Switch | Reproducer | Codegen result | Execution result |
|---|---|---|---|
| `CCC_NO_REGALLOC=1` | `ra_folded_index_wave_follow.c` | changed assembly | default and kill builds exit 0 with identical stdout |
| `CCC_NO_SEGMENT_SCAN=1` | `ra_folded_index_wave_follow.c` | changed assembly | default and kill builds exit 0 with identical stdout |
| `CCC_NO_FOLDED_INDEX_LIVENESS=1` | `ra_folded_index_wave_follow.c` | changed assembly | default and kill builds exit 0 with identical stdout |
| `CCC_NO_IR_DIVREM=1` | `divrem_pair_opposite_flavour.c` | changed assembly | default and kill builds exit 0 with identical stdout |
| `CCC_NO_MACHINST=1` | `ra_folded_index_wave_follow.c` | changed assembly | default and kill builds exit 0 with identical stdout |
| `CCC_NO_MULACC=1` | i686 `mulacc_chain_u64.c` | changed assembly | assembly smoke (i686 execution is separately environment-dependent) |

For a directly selected MachInst function, `CCC_MI_DEBUG=1` reports `enabled=true` normally and `enabled=false` with `CCC_NO_MACHINST=1`, confirming the constructor-owned setting reaches the selection gate. Full outputs/hashes are retained in `/home/user/artifacts/ms08/kill-switch-behavior-final/`.

## Correctness gates and same-window control measurement

The final compiler binary was built with the repository fastbuild preset (`-O1`, `-j2`) and the 8 GiB swapfile was active. The result gates are:

| Gate | Final result |
|---|---|
| `cargo check --profile fastbuild --lib -j2` | pass |
| focused `ra_config_tests` | 4/4 pass |
| complete `cargo test --profile fastbuild --lib -j2` | 1,992 passed; 0 failed; 6 ignored |
| `scripts/run_regression_suite.sh` with IR validation | `PASS=638 FAIL=0 SKIP=7`; A/B failures 0; boot tree honestly skipped |
| `scripts/check_benchmark_outputs.sh` | `PASS=156 FAIL=0 SKIP=0` |

A same-window, CPU-0-pinned, randomized-order A/B control compared the pristine baseline binary (the harness labels its historical `ccc` arm) to the final LCCC binary. Both arms produced matching outputs for all 11 timed rounds on zlib-ng Adler-32, gzip CRC-32, Expat XML scan, SQLite varint, and glibc memcmp. Because the default assembly is identical, these figures are a sanity control rather than a performance claim:

| Workload | Final median | Baseline median | Final / baseline paired median |
|---|---:|---:|---:|
| gzip CRC-32 | 151.22 ms | 151.78 ms | 0.9993 |
| zlib-ng Adler-32 | 41.44 ms | 41.41 ms | 1.0014 |
| Expat XML scan | 53.19 ms | 53.25 ms | 1.0005 |
| SQLite varint | 31.13 ms | 31.10 ms | 1.0005 |
| glibc memcmp | 10.09 ms | 10.06 ms | 1.0026 |
| geometric mean | — | — | **1.0009** |

The 95% bootstrap intervals include 1.0 for every control measurement. This is a shared KVM guest with no `perf` executable/usable PMU, so it is explicitly VM screening evidence, not a hardware-cycle claim. Raw rounds, commands, binaries, and reports are under `/home/user/artifacts/ms08/same-window-ab/`.

## Compiler Explorer inspection

The external oracle is unchanged by construction, but it was still rerun on the folded-index stress function to retain an independent code-quality view. For `emit_digits` in `ra_folded_index_wave_follow.c` at `-O2 -march=x86-64-v3`, final LCCC emits 46 instructions, 8 loads, 1 store, 0 spills, and 5 branches. Compiler Explorer reports GCC 16.2 at 38 instructions (the best count), ICC 2021.10 at 41, Clang 23.1 at 121, and current ICX at 63. The absence of an LCCC before/after count delta follows directly from the strict assembly identity gate; this work did not claim to close the independent 46-to-38 gap.

The manifest and source-specific report are at `/home/user/artifacts/ms08/godbolt-final/`.

## Follow-up

1. **Proceed with TASK-RA-06A only after this landing.** It shares `regalloc.rs`; its reload-at-use work must start from this explicit configuration boundary and re-measure the post-RA-33 residual spill shape before proposing policy changes.
2. **Do not use MS-08 as justification to default-enable experiments.** `CCC_EVICT_MODE`, `CCC_PGO_WEIGHT_MAX`, phase-4, MachInst controls, and every `CCC_NO_*` switch retain their previous default. A better static count or one local test is not a runtime-policy argument.
3. **Keep new RA/backend bisection knobs in `RaConfig` from their first landing.** Add the field comment, parser contract, injected-source test coverage, explicit driver thread, default identity gate, and a targeted behavior smoke together. Do not recreate process-global `OnceLock` configuration in a hot path.
4. **MS-09 remains open.** The peephole UTF-8 audit is separate from the small config-aware phase-4 plumbing here and still needs its dedicated evidence report.
