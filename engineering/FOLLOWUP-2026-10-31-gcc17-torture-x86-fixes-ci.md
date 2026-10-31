# 2026-10-31 — GCC 17-git x86 torture, codegen fixes, and CI handoff

## Executive result

Validated an upstream-main-based change set for the native x86-64 and i686
GCC `gcc.c-torture/{execute,compile}` campaigns. It contains two compiler fixes
with regression tests, the GCC 17-git provision/run wrapper and scheduled
GitHub Actions workflow, and the 180-second compile timeout adjustment. The
work is correctness-first: the signed narrow-vector division/remainder defect
was reproduced and fixed; the oversized x86 stack-adjustment immediate was
fixed and directly compiled. No runtime or end-to-end performance improvement
is claimed.

The current tested candidate has 0 LCCC failures in either full execute leg.
The full compile legs still expose real LCCC implementation gaps; these are
reported as compile failures, not hidden as skips. Reference failures,
reference compile skips, and unsupported tests remain separate categories.

## Provenance and reproduction

- Upstream `ms178/lccc` main was rechecked on 2026-10-31 and still resolves to
  `219f74aeb7cf77659b7fd4f0e8e9dd03a3a5b38e`, the base of this work. The
  candidate compiler source head used in the full reports is
  `9fbd5255c20b28c95217dbbdc86b8197e7f3801a`. The subsequent working-tree
  delta changes only the wrapper's default timeouts and its README entry.
- GCC development checkout: `c94b688a83fc84015c6fab8ab0ff142051598fc8`
  (`gcc-17.0.0-git-c94b688a83fc84015c6fab8ab0ff142051598fc8`). GCC upstream
  master was rechecked at the same SHA at campaign close.
- The runner selected 1,700 execute cases and 2,016 compile cases. The corpus
  contains 1,698 top-level execute C files; the runner's test-case count can
  exceed source count due to its case construction. The compile corpus has
  2,016 selected files. Reports include the selected-case/source digest and
  corpus stamp.
- Local comparison GCC is Debian GCC 14.2.0, not the development checkout.
  Thus `reference-fail` outcomes are oracle failures on this local version and
  must not be counted as LCCC failures. The CI workflow provisions GCC 16 and
  has a separate oracle/multilib verification step.
- LCCC was built with the repository `fastbuild` preset, opt-level 1, and
  exactly two Cargo jobs. The x86-64 full-run report records compiler SHA-256
  `b27d6474d3cde0dbeca4fcbfc6ef26c3e3dc9af555f2806139473d611222bbb6`;
  runner SHA-256 `4ff6419453de25a5e6d4d4779e6a99de791cd5b2e0f49020f2cd635645e9a737`.
- GCC corpus acquisition is reproducible via
  `python3 scripts/ensure_gcc_torture_git.py --ref c94b688a83fc84015c6fab8ab0ff142051598fc8`.
  Full wrapper usage is documented in `scripts/README.md`; normally run
  `bash scripts/ci_gcc17_torture.sh x86_64` or `... i686`.

## Full native matrix (`-O2`, two workers)

Execute legs link with the standalone `lccc-ld`; compile legs test assembly
production. Each report was complete (all expected cases). Compile timeout was
180 seconds and execute timeout was 10 seconds.

| Target / leg | Cases | Pass | LCCC compile-fail | LCCC run-fail | Reference compile-skip | Reference-fail | Unsupported |
|---|---:|---:|---:|---:|---:|---:|---:|
| x86-64 execute | 1,700 | 1,688 | 0 | 0 | 1 | 6 | 5 |
| i686 execute | 1,700 | 1,686 | 0 | 0 | 4 | 5 | 5 |
| x86-64 compile | 2,016 | 1,949 | 49 | n/a | 14 | 3 | 1 |
| i686 compile | 2,016 | 1,942 | 48 | n/a | 22 | 3 | 1 |

The 180-second x86-64 compile result supersedes the earlier 60-second run for
current status: the latter showed 52 compile-fail and included three test
timeouts. `limits-blockid.c`, `limits-externalid.c`, and `pr46534.c` all passed
on isolated 180-second retries; the final complete run also finished without
those timeout failures. It took about 240 seconds elapsed with two workers.
The 180-second i686 compile leg completed in about 225 seconds and had no
reported timeout cases.

Full reports and failure logs are durable under `/home/user/artifacts/`:

- `gcc17-x86_64-execute-o2-final.{json,failures.txt}`
- `gcc17-i686-execute-o2.{json,failures.txt}`
- `gcc17-x86_64-compile-o2-180.{json,failures.txt}`
- `gcc17-i686-compile-o2.{json,failures.txt}`
- `gcc17-deep-compile-recheck.json`

The full execute counts sum to all 1,700 cases. The six x86-64 reference
failures are `pr118623.c`, `pr118915.c`, `pr119291.c`, `pr122943.c`,
`pr123864.c`, and `pr124358.c`; i686 has the same set except
`pr118915.c`. GDB 16.3 directly confirmed that the local GCC 14.2 `-O2`
reference for `pr118623.c` aborts inside `bar`; GCC `-O0` and LCCC `-O2`
return successfully, and GCC's generated branch reaches the abort path for
this test. This is concrete evidence not to misattribute that reference
failure to LCCC. The compile-leg reference failures are GCC 14.2 ICE/failure
cases `pr117358.c`, `pr123365.c`, and `pr123703.c` on both architectures.

Reference compile skips and unsupported statuses remain visible in the JSON
reports. The execute unsupported set is `memclr.c` and `memcpy-a{1,2,4,8}.c`.
The compile unsupported case is `limits-fnargs.c`. These are not counted as
passes or as candidate failures.

## Validated compiler fixes

### 1. Signed narrow integer values crossing the x86 accumulator domain

A signed 16-bit vector divide/remainder is scalarized into x86 scalar division.
An `i16` value produced by a sign-extension-to-32 instruction can have the
correct low 16 bits and the correct signed low 32-bit representation while
its high half of the 64-bit GPR is zero. Passing that home directly to the
64-bit divide path changes a negative numerator (for example `-1`) into a
positive 65535 and gives an incorrect quotient/remainder.

The fix normalizes i8/u8/i16/u16 values when moving from a register home or
accumulator cache into `%rax`/`%rcx`, and when a narrow value already resides
in the scratch register. Signed widths use sign-extension instructions;
unsigned widths use zero extension. Typed aliases for `%al`/`%ax` and
`%cl`/`%cx` are now explicit. Type guards keep non-narrow values on the
existing full-width path.

Regression: `tests/regression/vector_sdiv_srem_short_signext.c` exercises both
signed quotient and remainder on negative and positive lanes. Its `.flags`
requests `-O2`. Before the fix this reproduced the incorrect sign behavior;
afterward the regression runner passed on x86-64 and i686 at `-O0`, `-O1`,
`-O2`, `-O3`, and `-Os`. The existing `vector-div-neg-pow2.c` also passed at
those levels on both targets. Generated code for the narrowed signed path was
inspected to confirm sign extension (`movswq`) precedes `idivq`; this is
instruction-shape evidence, not a performance benchmark.

### 2. x86 stack adjustment beyond signed imm32

The ordinary x86-64 `add/sub r64, imm` form accepts only a sign-extended
32-bit immediate. Large legal frames previously reached an immediate emitter
with an unencodable stack delta. The common small-frame immediate path remains
unchanged; larger positive deltas are materialized in caller-saved `%r11` and
applied with register-form `addq`/`subq`. The shared helper is used for frame
allocation and restoration paths, including the non-FPO allocation path.

Regression: `tests/regression/large_stack_frame_codegen.c` forces a
`0x10000000000`-byte automatic frame while keeping the function visible and
never calling it. The dedicated `large_stack_frame_codegen` regression passed.
GCC compile cases `20031023-{1,2,3}.c` and `stack-check-1.c` passed after the
fix; a four-case targeted stack-adjust validation passed. Existing FPO and
push/pop regression paths remained covered by `scripts/ci_local.sh --fast`
(see validation status below).

Scope boundary: `20031023-4.c` remains a distinct LCCC compile failure because
its frame access uses displacement `0x80000007`, outside signed disp32. This
patch fixes stack-pointer adjustment range; it does not claim to implement
large frame-offset address synthesis.

## CI design and validation

Added `.github/workflows/gcc17-torture.yml` and
`scripts/ci_gcc17_torture.sh`. The workflow runs manually, weekly, and on
relevant main-branch compiler/harness changes rather than adding the several
minutes of full corpus execution to the PR critical path. A build job produces
one fastbuild compiler artifact; separate x86-64 and i686 jobs download the
same artifact and verify both its commit identity and file hashes. The torture
jobs provision/verify GCC 16 plus multilib, run both execute and compile legs,
and upload JSON reports and failure logs even when a leg fails. Build and test
resource settings hold Cargo to two jobs and each test runner to two workers.

Validation completed: `actionlint v1.7.12` passed on the new workflow;
shell syntax, workflow/matrix assertions, `cargo fmt --all -- --check`, and a
shared-file wrapper smoke filter `^20000224-1\\.c$` passed. The smoke filter
selected the same source in both execute and compile legs. The initial smoke
filters selected no cases (one source was execute-only and one regex omitted
`.c`); they were corrected and are not represented as successful evidence.

`bash scripts/ci_local.sh --fast` was attempted. The environment lacks the
required `gcc-16`; oracle-dependent gates failed with `FileNotFoundError`,
including `zlib-compare256-correctness`, and native packed-FP/subscript
reference checks. The run reached `cargo-test` and was stopped before
completion to prioritize the requested immediate delivery. It is not a passing
CI result, no `ci_local.pass` is claimed, and the snapshot is explicitly
UNGATED. Re-run the fast gate after provisioning GCC 16 before treating this
checkpoint as a CI-cleared delivery.

## Known gaps / prioritized follow-up

1. The 49 x86-64 and 48 i686 compile failures are still candidate failures,
   primarily front-end parse/semantic/lowering gaps, inline-assembly constraints,
   unsupported options/features, and remaining backend/assembler limitations.
   The full `.failures.txt` files are the authoritative per-test inventory;
   do not aggregate them into a single unsupported bucket or silently filter
   them out. Prioritize the frequently shared front-end/codegen gaps, then
   introduce narrowly justified unsupported classifications only with an
   explicit reproducible reason and policy test.
2. Fix large frame-offset lowering independently of stack adjustment. Keep the
   `20031023-4.c` displacement case as a distinct regression target.
3. Run the same pinned corpus on GCC 16 in CI to replace the local GCC 14.2
   reference eligibility baseline; compare per-test identities, not only totals.
4. Full native top-level C torture is not complete DejaGnu compliance: this
   harness does not cover all target directives, nested suites, `.S`, LTO,
   diagnostic suites, or every option combination. Keep that limitation in
   future reports.
5. The sandbox has a shared virtualized CPU; no physical PMU/performance
   benchmarking is claimed. Preserve the existing fastbuild / two-job policy
   and do not interpret shared-host wall time as target performance.

## Delivery integrity gates

Before publishing a fresh snapshot/patch: wait for the `--fast` process and
record its exact result; verify the final working tree diff and whitespace;
check upstream `main` again; regenerate `/home/user/ms178-1.patch` and a named
snapshot from the validated tree; hash both; and inspect the staged patch for
secret/unrelated artifacts. Earlier S01/S02 snapshots are historical and
ungated. Never report them as the final deliverable.
