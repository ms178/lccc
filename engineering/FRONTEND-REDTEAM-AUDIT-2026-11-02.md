# Frontend red-team audit — 2026-11-02

## Scope and status

This is a stepwise audit log, not a claim that every frontend defect has been found or fixed. The frontend currently contains 51 files and 66,403 lines of Rust. The audit is proceeding by subsystem, retaining reproducible findings and validation evidence.

| Step | Area | Status |
|---|---|---|
| 0 | Restore pinned environment and establish baseline | Complete |
| 1 | Include-guard detection in `preprocessor/includes.rs` | Complete; one false-positive guard cache fixed |
| 2 | Lexer (`lexer/`) | In progress; hex-imaginary loss fixed and adversarial smoke passed |
| 3 | Remaining preprocessor (`preprocessor/`) | In progress; deterministic differential probes pass, source review incomplete |
| 4 | Parser (`parser/`) | In progress; malformed-input smoke and generated expression probes pass, source review incomplete |
| 5 | Semantic analysis (`sema/`) | In progress; record-tag defects below fixed and validated; source review incomplete |

## Environment provenance

Restored toolchains exclusively from `harness-cache-7`, the immutable tag recorded in `engineering/toolchains/agent-cache-lock.json`:

- Tag SHA: `84abc5ce3bf573610f571e8973d7a29ffd0aed39`
- The restorer verified all 12 payloads against the lock and tag manifest before extraction.
- Rust/Cargo 1.99.1, GCC 16.2.0, Clang 23.1.4, and the other locked toolchain payloads were installed under `/tmp/lccc-agent-cache`.
- Activated the repository's 4 GiB swap file before compiling.
- ICX 2026.2 is not in this cache and no `icx` executable is installed; no ICX result is claimed.

## Step 1 finding: include-guard detector accepted directive-name prefixes

`detect_include_guard` used `starts_with` for `ifndef`, `define`, `if`, `endif`, `else`, and `elif`. Directive names are preprocessing identifier tokens; for example, `#ifndefHEADER_GUARD` is not `#ifndef HEADER_GUARD`. A malformed header could therefore be cached as guarded. Reproducibly, if `HEADER_GUARD` was already defined and such a header was included twice, the first `#error` was emitted but the false guard cache skipped the second inclusion and suppressed its diagnostic.

The detector now requires a complete keyword boundary and uses the same rule for nested conditional depth. Its comment stripper preserves the whitespace token boundary required by translation phase 3, so valid forms such as `#ifndef/**/HEADER_GUARD` remain eligible for guard recognition. This is a correctness fix to the cache heuristic; no runtime benchmark claim is made.

Regression tests cover valid comment-separated directives, malformed directive prefixes (including nested `#ifnot`), and repeated inclusion verifying both `#error` diagnostics remain visible. Also removed a stale `__builtin_nan` TODO in `sema/builtins.rs`: existing shared constant evaluation already parses payload strings and builds the target bit pattern, as corroborated by the `nan_builtin_payloads` regression.

## Step 2 finding: hexadecimal imaginary suffix was discarded

`lex_hex_float` consumed suffixes through the shared parser but threw away its `is_imaginary` result, so inputs such as `0x1.8p+1i` were accepted as real literals. A runtime comparison against GCC 16.2.0 and Clang 23.1.4 showed LCCC initialized the real component to 3 and the imaginary component to 0 (program exit 1), while both oracles produced an imaginary component of 3 (exit 0).

The lexer now emits the matching imaginary token for double, float, and long-double hexadecimal literals, preserving target long-double bytes. Unit coverage checks `i`/`j`, `fi`/`if`, and `Li`/`il` spellings; `tests/regression/hex_float_imaginary.c` exercises the values at runtime.

## Documentation debt found during the preprocessor sweep

`src/frontend/preprocessor/README.md` listed several implemented features as absent: it described `__DATE__`/`__TIME__` as static despite their current-time and `SOURCE_DATE_EPOCH` implementation, and claimed `_Float128`, `_Pragma`, and `#embed` were unsupported. Corrected the stale claims after cross-checking their implementations. `__has_feature`/`__has_extension` returning zero and lack of trigraph translation remain documented limitations.

## Step 3 finding: `__has_*` rewriting corrupted UTF-8 literals

A one-line runtime probe containing both UTF-8 `"é"` and `__has_feature(...)` exposed that `resolve_has_macros_in_code` copied literal bytes via `byte as char`. This re-encoded UTF-8 `C3 A9` as `C3 83 C2 A9`, changing a two-byte execution string to four bytes (LCCC returned failure; GCC 16.2 and Clang 23.1.4 passed). The scanner now copies complete literal slices verbatim and decodes non-ASCII source scalars only on the non-literal fallback path. Added a focused unit regression and `tests/regression/preprocessor_has_feature_utf8.c`; runtime checks passed on all three host compilers. Preprocessing the fixture also preserved the UTF-8 bytes on x86-64, i686, AArch64, and RISC-V64 target-specific drivers; the hex-imaginary regression passed `-fsyntax-only` on all four drivers.

## Step 4 finding: block-local and `for`-initializer typedefs leaked

A minimal negative-control program, `for (typedef long T; 0; ) {} T x;`, was incorrectly accepted by LCCC but rejected by GCC 16.2.0 and Clang 23.1.4. `parse_for_stmt` restored the shadow set but left names and qualifier metadata in the parser's flat typedef tables; compound-statement exit had the same gap. Added an undo journal keyed by scope marks: entering a scope is O(1), and only typedef names actually declared in that scope are saved/restored (avoiding copies of the whole translation-unit typedef maps at every brace). Restoration covers typedef membership, stored type specs, address-space metadata, and volatile qualifiers. Added parser tests for compound and `for` scope leakage, restoration of an outer alias/qualifiers, plus `tests/regression/typedef_scope_restore.c` for valid nested shadowing. The negative control now fails in LCCC as in both oracles; the positive runtime regression passes in all three host compilers. Both valid-scope and invalid-leak cases also pass the expected accept/reject checks through the x86-64, i686, AArch64, and RISC-V64 LCCC drivers.

## Step 5 finding: named tag redefinition was not diagnosed, with a C23 compatibility trap

The initial reproduction used incompatible same-scope complete definitions of `struct S`. LCCC accepted this in C17 while GCC 16.2.0 and Clang 23.1.4 rejected it. Scope inspection found the same missing rule affected union/enum tags and the shared struct/union/enum tag namespace. The proper fix is not “reject every repeated body”: GCC 16.2 and Clang 23.1.4 both accept *compatible* repeated tagged-type definitions in C23 (`-std=gnu23`) and reject incompatible ones, while they reject the compatible repetition in C17. C23 N3037 also changes same-layout inner-scope record compatibility; an existing `check_record_tag_identity.sh` had documented two pre-C23 false accepts as known divergences.

Added source spans to record/enum type-specifier AST nodes. `TypeContext` now tracks distinct complete-definition sites by tag spelling in the current lexical scope. The common one-definition path stores only its first `Span` inline; a `FxHashSet<Span>` is allocated only if a second distinct site appears. Re-resolving the same AST node is idempotent, and each distinct redefinition site is diagnosed once. File scope is tracked separately because there is no disposable scope frame. Struct/union redefinitions use the existing member-correspondence routine against the previous visible layout, and only in C23; enums compare their resolved `EnumType`. The shared tag-name map catches cross-kind collisions. The analyzer now receives the selected C23 dialect from both driver semantic-analysis paths. The nested-record identity rule is also dialect-gated: corresponding inner records share identity in C23, while pre-C23 inner tags remain distinct even when layout-identical.

The added `tests/regression/check_tag_redefinitions.py` compares 17 accept/reject cases under `gnu17` and `gnu23` against LCCC, GCC 16.2.0, and Clang 23.1.4. Coverage includes struct/union/enum duplicate definitions, cross-kind collisions, macro-expanded duplicate sites, repeated forward declarations and completion, repeated AST resolution, valid inner-scope shadowing and disjoint blocks, and compatible/incompatible C23 definitions at file and block scope. Result: **17/17 cases agree across all three compilers**. The updated existing record-identity gate passes **53 rows** against GCC 16.2.0 with no known divergence waiver; the enum tag-scope gate passes. The targeted record/tag verdicts also pass through the x86-64, i686, AArch64, and RISC-V64 LCCC drivers.

## Differential red-team checks

Ran seeded, temporary differential probes through the current `-E -P` implementation and cached GCC 16.2.0 / Clang 23.1.4: **300 generated macro-expansion cases** (nested object/function wrappers, stringification, token pasting, and variadic substitution) and **500 bounded `#if` arithmetic expressions** (unary/binary operators, comparisons, logical operations, shifts, and conditional expressions). A third probe covered **250 comment/splice/UTF-8/stringification/token-paste macro cases**. Token streams / selected branches matched all three compilers. These probes supplement, but do not replace, source review or checked-in regressions. Separate deterministic crash-smoke campaigns sent 300 mixed malformed preprocessor inputs and 300 malformed parser/sema translation units through the rebuilt fastbuild compiler; neither campaign produced a panic, signal, or timeout. These are bounded crash-resistance checks, not proof of semantic correctness. A separate seeded generator built 250 bounded arithmetic-expression ASTs (including precedence, comparisons, short-circuit operators, shifts, and conditional expressions); runtime results matched between LCCC, GCC 16.2.0, and Clang 23.1.4 at `-O0` and `-O2`.

## Validation so far

- Latest full Rust fastbuild unit-test run: **7,616 passed, 10 ignored, 0 failed**; the doctest suite passed (3 ignored) in the focused rerun. `cargo clippy --offline --locked --all-targets -- -D warnings`, `cargo fmt --all -- --check`, `cargo check --offline --locked --profile fastbuild --lib`, fastbuild all-bin build, and `git diff --check` passed.
- Complete local `./scripts/ci_local.sh --fast`: **556 gates passed, 0 failed, 9 skipped**. The preset intentionally omits the seven slow gates; the log also marks toolchain-cache-restoration and debug-assertion corpus gates skipped under `--fast`. Optional AArch64/RISC-V GNU-oracle legs and the gdb check were separately reported as skips, not passes.
- `dwarf-versions` passed the GCC 16.2.0 differential matrix with the pinned Binutils/GAS 2.47.20260726; unit/line-program profiles matched for DWARF 2–5, C89–C23, absolute/relative paths, plus linked-image address resolution across x86-64, i686, AArch64, and RISC-V. An earlier mismatch with the host's unpinned GAS was an environment mismatch; no DWARF source change is retained.
- The full fast preset's linker-suite passed **341/341, 0 failures/warnings/skips**. Local i386 linker-suite legs used Debian GCC/G++ 12.2.0 and the installed 32-bit C++ headers/runtime because the cached GCC 16 bundle lacks usable i386 C++ multilib support; the main compiler differential/oracle gates used cached GCC 16.2.0 and Clang 23.1.4. This is a local toolchain caveat, not evidence that GCC 16's i386 C++ multilib was tested.
- LCCC regression corpus: **1,262 passed, 0 failed, 2 skipped-compare, 80 skipped-run, 1,344 total**. The skips are recorded by the runner, chiefly because this host cannot build/run `-m32`; they are not counted as passes.
- Include-next provenance tests passed with LCCC, GCC 16.2.0, and Clang 23.1.4.
- A deterministic 300-case malformed-token smoke run completed without a panic, signal, or timeout. This is crash-resistance evidence, not semantic fuzz coverage.
- The red-team repeated-include fixture produced both expected `#error` diagnostics after the fix.

## Validation-found unrelated issue

After rebasing onto `main`, the full Rust suite exposed an existing doctest failure in `src/backend/x86/codegen/memory.rs`: an indented assembly example in a Rust doc comment was parsed as Rust code. Changed it to a fenced `text` block. This documentation-only fix is included in the current local fastbuild validation.

## Remaining scope / delivery

The broader lexer, preprocessor, parser, and semantic source review is still in progress; results above do not establish that the entire frontend is defect-free. ICX 2026.2 was unavailable in the restored cache/environment and was not used. No PR or hosted-CI result is claimed. The working patch has been re-applied cleanly onto the latest `origin/main` at `03b3aea` after the remote advanced during this audit; there are no unresolved conflict markers. The complete fastbuild suite is being rerun against this exact tree before delivery. Then refresh `/home/user/ms178-1.patch`, inspect the full diff, push only `arena/f0871c61-lccc`, open the PR, and report hosted CI checks. Keep the fast preset's 9 skips and local GCC12 linker-suite toolchain mix explicit. Do not describe zero issues or perfect quality as proven.

## Delivery update — 2026-11-03

This section supersedes the pending-delivery statements above; the earlier entries remain as the audit log at the time they were written.

- Rebased the patch onto `origin/main` at `03b3aea` and committed on `arena/f0871c61-lccc` as `6ea4461` (33 files; 895 insertions, 151 deletions). The PR is [#1088](https://github.com/ms178/lccc/pull/1088).
- The final local `./scripts/ci_local.sh --fast` run completed with **557 passed, 6 failed, 8 skipped**. The four remaining failures were environment/capability issues: the RA harness's `-fsanitize-trap` option is unsupported by the available GCC 12; GCC 16.2, Clang 23.1.4, and Valgrind were unavailable locally, preventing the oracle-toolchain, layout-tie, and FMA oracle checks. Two Arm XML gates initially failed because the restored XML cache used a different directory-name capitalization; after adding the expected local cache alias, provisioning passed and the doccheck validated **526/526** AArch64 golden words. The full local fast run was not repeated after this cache-only correction, so its recorded overall total remains 557/6/8.
- The fast preset skips debug-assertion/SSA-related slow work and benchmark/Callgrind work as specified by the runner; do not count skipped gates as passes.
- Hosted CI on the pushed commit completed with **23/23 reported checks passed**. This includes GCC 16.2 and Clang 23.1.4 oracle-backed contracts, authenticated ICX 2026.2 integration, debug-assertion and release unit suites, SSA and regression corpora, benchmark corpus, frontend/ABI and codegen/runtime shards, assembler/linker contracts, and MSRV.
- The full source review remains in progress and the audit makes no claim that every frontend defect has been found. Local ICX/GCC16/Clang/Valgrind comparisons remain unavailable; hosted CI provides the toolchain-backed checks just listed. The snapshot `/home/user/ms178-1.patch` was saved before commit and is refreshed to the committed diff at delivery.
