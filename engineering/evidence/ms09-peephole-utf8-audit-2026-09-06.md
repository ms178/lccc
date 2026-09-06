# MS-09 — peephole UTF-8 provenance audit and live repair

**Date:** 2026-09-06

**Status:** closed — historic source exposure audited; a live inline-assembly
text conversion and an independent x86 text-rewrite reconstruction defect are
repaired and regression-gated.

**Audit base:** `7bd43d0999ea90e0ef71d3cf5781dee8f01c648e` (descendant of
`origin/main` `aae69e2b019a313afd0be125c6727ed6078c95f4` at audit time).

## Executive conclusion

There are two distinct conclusions, and neither should be overstated:

1. **Historic semantic exposure is proven.** Before the 2026-08-18 repair,
   the shared textual peephole helper rebuilt unmatched bytes with
   `bytes[i] as char`.  The exact historical helper, extracted verbatim from
   `331f39c7^`, demonstrably turns UTF-8 text such as `café € 🦀` into
   UTF-8-encoded Latin-1 mojibake.  Before the 2026-02-05 consolidation,
   ARM and RISC-V had equivalent architecture-local helpers.  Consequently, a
   compiler built from an affected revision **could** emit corrupted UTF-8
   when such a textual rewrite processed that content.
2. **No tracked official public binary distribution was found, not no
   distribution exists.** The online GitHub audit found zero public releases,
   zero tags, no retained Actions artifacts, and no historical workflow upload
   that named a compiler.  This supports: *no official GitHub release/tag or
   retained Actions compiler artifact carrying this bug was evidenced on
   2026-09-06.*  It does **not** prove that a private CI run, manual archive,
   developer build, fork, deleted/expired artifact, or another distribution
   channel never supplied an affected executable.

This is deliberately a **textual-peephole and inline-asm transport audit**.  It
is not a claim that every compiler path, arbitrary byte parser, or every
historic LCCC binary was UTF-8 safe.

## Scope and method

The machine-readable historical/publication audit is
`/home/user/artifacts/ms09/peephole-utf8-audit.json` (`status: PASS`).  It is
produced atomically by:

```sh
python3 scripts/audit_peephole_utf8.py --online \
  --json /home/user/artifacts/ms09/peephole-utf8-audit.json
```

The script makes the following bounded checks:

* exact historical source shape at the shared fix and its parent;
* the pre-consolidation ARM and RISC-V peephole copies;
* every current architecture-independent/ARM/RISC-V/i686/x86 textual
  peephole source for the dangerous operation of appending an indexed raw byte
  as a Rust `char` to an output string;
* historic workflow artifact steps, current policy wording, and ancestry of
  the shared repair; and
* in `--online` mode, the public `ms178/lccc` GitHub metadata, releases,
  tags, workflows, and paginated Actions-artifacts endpoints.

The audit retains source hashes and the full workflow/upload evidence.  Its
current source scan found **zero** scoped raw byte-to-char reconstruction
sites.  Contexts which merely inspect a byte are retained for review and are
not mislabeled as output reconstruction.

## Historic timeline and exact semantic reproduction

| Period / revision | Finding | Consequence |
|---|---|---|
| Before `3494fd6e` (2026-02-05) | `src/backend/arm/codegen/peephole.rs` and `src/backend/riscv/codegen/peephole.rs` each contained the vulnerable rebuild loop. | ARM and RISC-V textual peepholes had the defect independently. |
| `3494fd6e` — “Consolidate duplicated backend code” | The helpers moved into `src/backend/peephole_common.rs`. | The one shared implementation served the architecture consumers; sharing reduced duplication but propagated the same semantics. |
| `331f39c7^` / `1f4be0a2` (2026-08-18 20:25:13 +02:00) | The shared `replace_whole_word` still used `result.push(bytes[i] as char)`. | Any unmatched non-ASCII bytes passed through this helper were recoded. |
| `331f39c7` (2026-08-18 20:37:54 +02:00) | The shared helper changed to UTF-8-preserving slice/byte splicing. | The historic shared issue was repaired and this commit is an ancestor of both audit base and `origin/main`. |

A full historical `cargo test` build was attempted but is not usable proof:
that old parent fails before test execution under the retained current
Rust toolchain due to unrelated `split_ranges.rs` type/borrow errors.  Rather
than turn that infrastructure failure into a claim, the audit uses a narrow,
reproducible semantic test:

```sh
python3 scripts/reproduce_historical_peephole_utf8.py \
  --save-source /home/user/artifacts/ms09/historical-shared-utf8-reproducer.rs \
  --json /home/user/artifacts/ms09/historical-shared-utf8-reproducer.json
```

The reproducer extracts **verbatim** `is_ident_char` and
`replace_whole_word` from `331f39c7^:src/backend/peephole_common.rs`, compiles
only those self-contained functions, and asserts byte-exact behavior.  It
passed and printed `REPRODUCED legacy_byte_recode`.  For the replacement
`x1 → x9`, expected `café € 🦀` bytes are:

```text
c3 a9 20 e2 82 ac 20 f0 9f a6 80
```

while the historical output contains the corresponding doubled encoding:

```text
c3 83 c2 a9 20 c3 a2 c2 82 c2 ac 20 c3 b0 c2 9f c2 a6 c2 80
```

This proves the extracted historic helper's semantics.  It does not claim a
full historic compiler built successfully, nor that a binary from it was
published.

## Public-distribution evidence and limitation

At the online audit time, GitHub reported the public repository as created on
2026-08-09 and returned:

| Publicly observable channel | Result |
|---|---:|
| GitHub Releases | 0 |
| Git tags | 0 |
| retained Actions artifacts | 0 |
| compiler-named retained Actions artifacts | 0 |
| workflows | 3 (Benchmarks, CI, Deploy GitHub Pages) |
| historical compiler-artifact upload candidates | 0 |

The only historic upload step found was `benchmark-results` in
`.github/workflows/bench.yml`; its paths were `bench-results.json` and
`bench-summary.md`, not a compiler binary.  The audit follows pagination and
handles GitHub's object-wrapped `artifacts` response, so this result is not an
accidental first-page or schema-only observation.

This establishes the limited, useful result that no official **tracked public
GitHub publication** was found.  It cannot establish absence of private/manual
or expired/deleted distribution.  The public repository being available during
part of the vulnerable shared-helper interval also means a user could have
built it from source; a source build is not evidence that a project-provided
binary was shipped.

## Live defects found and repair rationale

The historical audit did not justify stopping at the old helper.  The
end-to-end probe exposed two live, independent byte reconstruction sites.

### 1. Frontend inline-assembly template transport

The lexer deliberately models a narrow C string as one Rust `char` per source
byte.  That is appropriate for byte-array initializers, including invalid
bytes.  Inline asm is different: the template is later appended to a UTF-8
assembly `String`.  Appending the narrow-byte carrier directly re-encodes each
high byte as Unicode Latin-1.

`narrow_string_byte_carrier_to_utf8` in `src/common/encoding.rs` converts only
when all carrier characters fit in `u8` **and** the reconstructed byte buffer
is valid UTF-8.  `parse_asm_string` now concatenates all adjacent C string
literals first and decodes once.  Concatenating first is material: a multibyte
character may cross C-literal boundaries.  Invalid UTF-8 or a non-byte carrier
retains the former `String` rather than silently introducing replacement
characters or discarding data.

### 2. x86 `replace_reg_name_exact`

`src/backend/x86/codegen/peephole/passes/helpers.rs` independently rebuilt an
entire assembly line one `u8 as char` at a time.  It therefore corrupted an
unmatched UTF-8 comment even when no register matched.  This was not exercised
by the inline-asm route: inline-asm regions are pinned before x86 copy
propagation.  It was nevertheless an exposed helper path and is repaired
separately.

The replacement now searches bytewise only for ASCII register spellings, checks
UTF-8 boundaries for a match, and copies all unmatched portions by `&str`
slice.  It returns the original allocation when no match is found or the
request is a no-op.  This keeps its exact-register delimiter behavior while
making text preservation explicit.  `engineering/agent/RULES.md` now bans raw
`bytes[i] as char` reconstruction in **any** peephole text-rewrite helper, not
only the old shared helper.

## Regression and generated-assembly proof

`tests/regression/peephole_utf8_inline_asm.c` contains both a normal literal
and one deliberately split as `"caf\\303" "\\251 ..."`, spanning 2-, 3-, and
4-byte UTF-8 code points.  It also performs an `addq` output operand operation
and returns failure unless `41` becomes `42`.

`scripts/check_inline_asm_utf8.py` is the non-vacuous byte-level checker.  It
requires the source markers and split fragments, compiles both `-S` and a
linked executable, validates the assembly UTF-8, isolates the `#APP`/`#NO_APP`
region, and checks the exact expected bytes and absence of the legacy mojibake
bytes.  JSON is atomically written.

| Binary under test | exact markers in `#APP` | legacy markers in `#APP` | compile/link/run |
|---|---:|---:|---:|
| saved pre-change binary (`80ac31…9dda`) | 0 / 2 | 2 / 2 | pass / pass / exit 0 |
| repaired `target/fastbuild/lccc` | 2 / 2 | 0 / 2 | pass / pass / exit 0 |

Artifacts are `/home/user/artifacts/ms09/inline-asm-baseline.{json,s,stdout}`
and `/home/user/artifacts/ms09/inline-asm-candidate-final.{json,s}`.  The
linked executables have the same SHA-256 because comments do not change object
code; this is why the assembly-byte assertion is necessary in addition to the
exit-code gate.

Focused Rust unit tests each executed exactly one test and passed:

* `narrow_byte_carrier_decodes_valid_utf8_once`, plus invalid/non-byte refusal;
* `replacement_preserves_unmatched_utf8_bytes`, including a no-register-match
  line; and
* strengthened shared `replace_whole_word_preserves_utf8` coverage.

The CI workflow runs the normal C corpus and now separately invokes the
byte-level checker against `target/release/lccc`, uploading
`inline-asm-utf8.json` with the regression result.  Thus comments cannot make
this test vacuously green merely because the executable still runs.

## Broader validation

All commands below passed with the required compiler build mode (`fastbuild`,
Rust `-O1`, two jobs):

| Check | Result | Retained evidence |
|---|---|---|
| `./scripts/build_lccc_fast.sh` | fastbuild (`-O1`, two jobs) succeeded in 23.91 s | `build-fastbuild.log` |
| `./scripts/build_lccc_o1_j2.sh` | release (`-O1`, two jobs) succeeded in 8m16s | `build-release-o1-j2.log` |
| release byte checker (`target/release/lccc`) | exact 2 / 2; legacy 0 / 2; linked program exit 0 | `inline-asm-release-final.{json,s,stdout}` |
| `cargo test --profile fastbuild --locked --lib -j2` | 1,996 passed; 0 failed; 6 ignored | `cargo-test-lib.log` |
| `CCC_VALIDATE_SSA=1 python3 tests/regression/run_regression.py --lccc target/fastbuild/lccc -j 2` | 665 passed; 0 failed; 13 GCC-oracle skips | `regression-full-ssa.{log,json}` |
| `python3 -m py_compile` on all three MS-09 scripts and the runner | pass | command log |
| targeted `rustfmt --check` on edited Rust files; `git diff --check` | pass | command log |

The full-tree `cargo fmt --check` is intentionally not claimed: unrelated,
pre-existing formatting differences make it fail.  Only the changed Rust files
were formatted/checked.

## Normal-codegen and runtime non-regression screen

This repair is intended to preserve text, not improve ordinary kernel code.
The checks therefore reject an accidental performance/codegen movement rather
than advertise a speedup.

* `scripts/check_codegen_refactor_identity.py` compared the saved pre-change
  compiler with the repair over all 16 `tests/benchmark/kernel_corpus` inputs
  at `-O2`: **16 IDENTICAL, 0 DIFFERENT, 0 COMPILE_FAIL**.  Full listings and
  canonical hashes are retained in
  `/home/user/artifacts/ms09/codegen-identity-kernel/report.json`.
* A randomized, CPU-0-pinned 15-round paired A/B compared the repaired binary
  against the saved pre-change binary on gzip CRC-32, zlib-ng Adler-32, Expat
  XML scan, SQLite varint, and glibc memcmp.  Every compile and checksum/output
  gate passed.  The candidate/baseline geometric paired-median ratio was
  **0.9986** (`<1` means candidate faster), which is no material performance
  movement.

| Workload | required output | candidate/baseline median ratio | bootstrap 95% CI |
|---|---|---:|---:|
| gzip CRC-32 | `372e56ab` | 1.000 | [0.999, 1.001] |
| zlib-ng Adler-32 | `8c331ae0` | 1.000 | [0.998, 1.001] |
| Expat XML scan | `626766774715194881` | 1.000 | [0.990, 1.008] |
| SQLite varint | `deedcdd4edc1c0f1` | 0.999 | [0.986, 1.002] |
| glibc memcmp | `2158787064` | 0.994 | [0.992, 1.006] |

The run used a two-vCPU KVM environment (Intel Xeon 2.60 GHz), randomized
compiler order, two warmups, no automatic outlier removal, and wall-clock plus
child CPU-time collection.  No `perf`/PMU was available, and the short memcmp
case is especially noisy.  It is a same-window **non-regression screen**, not
a Raptor Lake speedup or microarchitectural claim.  Raw rounds, binaries, and
method metadata are in `/home/user/artifacts/ms09/ab-runtime/` and
`ab-runtime.{json,md}`.

## Compiler Explorer comparison

The first attempt used a static Expat helper at normal `-O2`; Clang, ICC, and
ICX inlined/removed it.  It is retained only as transparency evidence at
`/home/user/artifacts/ms09/godbolt-expat-utf8/` and is **not** counted as a
four-oracle comparison.

Two successful, non-vacuous comparisons then ran via `scripts/godbolt.py`:

1. `engineering/evidence/godbolt/ms09-inline-asm-utf8.c` declares an external
   `__attribute__((noinline))` inline-asm companion.  GCC 16.2, Clang 23.1.0,
   ICC 2021.10.0, and current ICX all emitted and were extracted successfully;
   instruction counts were LCCC 6, then 3 for each oracle.  This validates the
   comparison plumbing and a universally emitted inline-asm shape; it is not a
   throughput claim.  Compiler Explorer's presentation strips asm comment
   delimiters, so the local byte checker—not remote displayed comments—is the
   authoritative text-preservation proof.
2. The representative `expat_utf8_name_length` was forced externally visible
   with `-O2 -fno-inline -march=x86-64-v3`: LCCC 80, GCC 16.2 72, Clang 62,
   ICC 64, ICX 63 instructions.  This is a known code-quality gap unrelated to
   a preservation-only change and is recorded rather than obscured.

The successful manifests and assembly are in
`/home/user/artifacts/ms09/godbolt-inline-asm-utf8/` and
`/home/user/artifacts/ms09/godbolt-expat-utf8-noinline/`.  Instruction counts
are triage signals only; they do not replace timing or establish an oracle's
implementation as a target.

## Follow-up boundary

MS-09 is complete for the stated historical textual-peephole/publication audit
and the discovered live repairs.  Future work should keep the guard and test
in place, but must open a separate audit with its own provenance and semantic
proof before making claims about non-peephole parsers, arbitrary binary
transport, private distribution, or compiler-wide Unicode behavior.
