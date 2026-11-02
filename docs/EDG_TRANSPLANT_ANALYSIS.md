# EDG C/C++ Front End → LCCC: Deep-Source Transplant Analysis

**Date:** 2026-10-01 · **EDG source:** https://github.com/edgcpp/compiler (cloned
2026-10-01, `--depth 1`, Apache-2.0 WITH LLVM-exception) · **Analyst goal:**
extract *everything of value* from the EDG code base for LCCC and turn it into
actionable, performance- and correctness-serving work items.

> **Audit boundary, 2026-10-02:** This is historical source/research analysis,
> not a compiler correctness, redistribution-clearance or runtime-performance
> certificate. The current independently tested corpus/recovery/tooling contract
> and unfinished work are in [the audit](../engineering/PR724-AUDIT-2026-10-02.md)
> and [corpus README](../tests/corpus/README.md). Source pin for the audited
> Clang subtree/Changes/notices: `0ac366374c06e54612ddbac397c74ffa40e5182c`.
> The broader LOC/tooling inventories below were not independently re-counted
> in this no-build audit. No compiler optimization or benchmark win is claimed.


---

## 0. TL;DR — the five transplants that matter most

| # | Transplant | Why it matters for LCCC | Tier |
|---|------------|-------------------------|------|
| 1 | **Clang-derived C test corpus** (`tests/tests/imported/clang/c/`, 1 529 `.sft.c` files: `C/`, `Parser/`, `Preprocessor/`, `Sema/`) | Immediate differential-correctness fuel for lccc's frontend; pinned subtree notice retained; per-file rights/oracle review still required | **T0** |
| 2 | **Fixed-geometry Cachegrind benchmark methodology** (`dev_tools/bin/edg-bench`, `edgbench` pylib) | Fixed modeled cache geometry for no-PMU triage; matched software stacks are still required and this is not target timing | **T0** |
| 3 | **Builtin-signature database pipeline** (`dev_tools/builtins/`, `generate-builtin-table.py` → `src/builtin_defs.h`, 308 K lines) | lccc's `sema/builtins.rs` is a hand-maintained name→libc map; EDG shows how to *scrape signatures from GCC/Clang binaries via plugin interfaces* (extraction/distribution review required) and generate a complete, versioned builtin table | **T1** |
| 4 | **Diagnostic catalog design** (`src/error_msg.txt` + `util/mk_errinfo.c` → `err_codes.h`/`err_data.h`, severity tags) | Single source of truth, stable codes, user-tunable severity tags, auto-generated docs — LCCC's `common/error.rs` diagnostics are free-form strings today | **T1** |
| 5 | **Constant-evaluation interpreter architecture** (`src/interpret.c` 34.8 K lines: explicit work stack, 4 storage pools, cost budget, diagnostic lists) | The reference design for C23 `constexpr`/`_Static_assert`/initializers and for hardening lccc's `const_eval.rs` against deep-expression stack overflows and uncompilable-input DoS | **T1** |

Everything else below is organized subsystem-by-subsystem with file-level
evidence, pros/cons, and a concrete recommendation. Nothing in this document is
"a nice idea" without an identified consumer inside LCCC.

---

## 1. What EDG actually is (and is not)

> "The front end does not do any optimizations. It does not transform the
> program in other than trivial ways."
> — `doc/source/int_overview.rst`

EDG is the industry-standard **C/C++ front end** (used inside many commercial
compilers; NVCC, Intel ICC's front end and others historically EDG-derived).
The open-sourced repository contains:

- **Front end** (preprocessor → parser → sema → **tree-shaped high-level IL**)
- **IL lowering** (C++ IL → C-like IL; `lower_il.c`, `lower_init.c`, `lower_eh.c`,
  `lower_c99.c`)
- **C-generating back end** (`c_gen_be.c`) and **C++-generating back end**
  (`cp_gen_be.c`) — i.e. "C as portable assembly"
- **Prelinker** (template instantiation), **IL file read/write/display**,
  **demangler**, **daemon example**, **extensive dev tooling and tests**

It contains **no SSA optimizer, no register allocator, no machine-code
backend**. Consequently the transplant value for LCCC's supreme objective
(generated-code performance) is *indirect but foundational*:

```text
EDG value for LCCC = frontend correctness/compatibility
                   + ABI/layout exactness
                   + constant-evaluation quality
                   + test/bench/dev methodology
                   + 34 years (1992–2026) of edge-case knowledge in
                     src/Changes (7.1 MB)
```

Better front ends feed better alias/range/const information into the optimizer,
which is exactly the §14/§16/§17 pipeline in our research charter. EDG is not
a machine-codegen competitor; its sources and historical changes are useful
correctness references. Emulation modes and heuristic C relevance are not ISO C
authority or executable-oracle proof.

---

## 2. Quantitative comparison

| Metric | EDG (`edgcpp/compiler`) | LCCC (`ms178/lccc`) |
|---|---|---|
| Language | C++11 in `.c` files (historical naming) | Rust 2024 |
| Core compiler LOC | **1 573 339** (`src/*.c`+`*.h`) | **674 208** (`src/**/*.rs`) |
| Largest files | `builtin_defs.h` 308 K (generated), `unicode_name_fsm.c` 69 K (generated), `expr.c` 60 K, `templates.c` 47 K | `passes/vectorize.rs` 28.6 K, `loop_unroll.rs` 8.3 K, `slp_vectorizer.rs` 6.3 K |
| Test files | 45 109 single-file tests in `tests/tests/` (19 717 `.sft.c`, 25 392 `.sft.cpp`): 2 925 EDG-authored (9 006 files with recorded outputs), 40 018 imported (95 752 files with outputs; GNU GPL-3 + Clang Apache-2.0), remainder in `modules/`, `cwg/`, `regressions/` | 720+ regression corpus, Csmith/yarpgen fuzz, GCC-torture drivers (`x86_gcc_torture.py`), + mined EDG corpora (this work: `tests/corpus/`) |
| Documentation | 47 494 lines of curated `.rst` (per-module: `il.rst`, `lower_il.rst`, `lex_pp.rst`, `expr.rst`, `err_msgs.rst`, `testing.rst`, `tables.rst` …) | `docs/` + per-module `README.md` + `engineering/` journal |
| Dev tooling | 985 MB `dev_tools/`: test runner, bisect, race-reducer (`edgy-reduce-race`), bench with Cachegrind fixed geometry, CE packer, IFC dumper, Unicode tools | 161 scripts: godbolt oracle, codegen oracle, A/B harnesses, snapshot machinery, kernel harness |
| Build targets | front end + C/C++ BEs + prelinker + daemon | full toolchain: 4 backends (x86-64/i686/AArch64/RV64), assembler, linker `lccc-ld`, PGO |
| License | Apache-2.0 **WITH LLVM-exception** (core); imported GNU tests **GPL-3** (tests only); imported Clang tests Apache-2.0+LLVM-exception | Apache-2.0 / MIT / BSD-3 / CC0-CCC (see `LICENSING.md`) |

**Architectural distinction:** EDG has no machine-code optimizer/backend to
rank against LCCC's code generation. LOC ratios do not establish superiority,
correctness or performance. Dialect/ABI/diagnostic/constant-evaluation machinery
and test/dev methodology are source-analysis opportunities, not measured wins.
Those are the proposed transplant seams.

---

## 3. EDG pros and cons

### Pros (why EDG is worth mining)

1. **Bug emulation as a first-class feature.** `$EDG/src/cmd_line.c` carries
   `emulate_gnu_abi_bugs`, `emulate_unsafe_gnu_abi_bugs`,
   `emulate_msvc_value_initialization_bugs`, versioned MSVC emulation tiers
   (`--ms_extensions` / `--ms_compatibility` / `--microsoft_version`),
   `--gnu_version` emulation, Sun CC emulation. Real-world source bases
   compile because EDG reproduces *competitor misbehaviour deliberately*.
2. **Preprocessor architecture that preserves provenance.** `lexical.c` +
   `doc/source/lex_pp.rst`: logical-line reconstruction via
   `orig_line_modif_list`, macro expansions recorded as *modifications* (not
   text rewrites), `ATTENTION_MARKER`/`LE_*` escape scheme, inert-macro marks.
   Column-accurate diagnostics through arbitrarily nested expansions.
3. **Generated, versioned builtin database.** `builtin_defs.h` is generated by
   scraping builtin *signatures* out of GCC/Clang executables through their
   plugin interfaces (see `dev_tools/builtins/README.txt`) — an extraction/distribution review point, not legal clearance; a
   method for maintaining signatures across ~20 compiler versions × 4 ABIs.
4. **Constant evaluator designed for hostile input.** `interpret.c`: explicit
   work stack (bounded call-stack), cost budget (`2·calls + loop-backs`),
   four storage pools (LIFO stack, per-invocation static, persistent, maps),
   pointer-validity tracking, diagnostic-list results instead of
   error/abort. This is production-grade constexpr evaluation.
5. **Correctly-rounded decimal↔binary FP.** `floating.c` implements
   Steele&White PLDI'90, Clinger PLDI'90 and Gay's corrected algorithm as a
   host-independent fallback (`USE_HOST_FP_CONVERSION_ROUTINES=FALSE`).
6. **Exact multi-word integer arithmetic.** `const_ints.c`:
   `an_integer_value` as `INT_VALUE_PARTS`-array with signed/unsigned ops —
   no i128-only shortcut; target widths independent of host.
7. **Layout engine with explicit ABI knobs.** `layout.c` implements
   declaration-order vs accessibility grouping, MS bit-field allocation
   (`targ_microsoft_bit_field_allocation`), negative
   `targ_bit_field_container_size` GNU variants, zero-width bit-field
   alignment quirks (`emulate_gnu_abi_bugs`), empty-base optimization,
   `[[no_unique_address]]`.
8. **Diagnostic catalog discipline.** `error_msg.txt` is the *only* place a
   message exists; `util/mk_errinfo.c` generates `err_codes.h` + `err_data.h`;
   stable numeric codes (never renumbered; retired entries marked
   `REMOVED;`); every diagnostic has a **tag** so users can
   `-Wno-tag`/`-Werror=tag`; docs generated from the same file
   (`doc/source/err_msgs.rst`, 10 449 lines).
9. **Test framework with expected-output recording.** `.sft` directives
   (`//type:`, `//options:`, `//cases:`, `//match_regex:`, `//require:`,
   `//filter:`), `TEST_NUMBER`-driven multi-variant tests,
   `edg-run-test --record` re-records expectations,
   `tests/expectations/<target>/` per-target golden output, plus
   `edg-normalize-test-output`, `edg-test-run-sonar`, `edg-bisect`,
   `edgy-reduce-race` (race-preserving test reduction!).
10. **Deterministic benchmarking without a PMU.** `dev_tools/bin/edg-bench`
    runs Valgrind **Cachegrind with a fixed, hard-coded cache geometry**
    (`I1=32768,8,64 D1=32768,8,64 LL=33554432,16,64`) precisely so results
    have a fixed modeled cache geometry; software/library/startup and ISA
    differences still require controls; `cg_annotate` post-processing,
    baseline directories (`benchmarks/baselines/project/<config>/`),
    `edg-bench-run-delta` for regression triage.
11. **Source-correspondence IL.** `il_def.h` carries `a_source_sequence_entry`
    chains (`src_seq.c`), `a_source_range`s on nodes, shareable-constant
    interning (`alloc_shareable_constant` + `hash_constant`/`eq_constants`),
    region-based IL allocation (`il_alloc.c` regions switched wholesale).
12. **IL serialization** (`il_write.c`/`il_read.c` with magic+version) and
    `il_display.c`/`il_to_str.c` human-readable dumps — the substrate for
    IL-diff tooling and PCH.
13. **PCH and daemon.** `pch.c` (precompiled headers via IL regions) and
    `cfe_daemon.c` (Unix-socket multithreaded FE-as-library server, with
    protocol in `cfe_daemon_common.h`) — compile-latency levers.
14. **Embedding API.** `doc/source/ext_intf.rst` (5 681 lines) documents the
    FE-as-library interface (`MAKE_FRONT_END_CALLABLE`), custom output files,
    option-string ABI. LCCC's driver could adopt the same "compiler as a
    service" shape later (godbolt/CE packaging — EDG ships `edg-pack-cpfe-ce`
    that packs `cpfe`+`edg_decode`+`edg_prelink`+`libedgrt.a` as a Compiler
    Explorer tarball!).
15. **34 years (1992–2026) of edge cases in `src/Changes`** (7.1 MB changelog with bug
    IDs, standard-paper references — e.g. WG14 N3037 tag compatibility, C23
    empty initializers dated 9/2026) — a searchable knowledge base of "what
    actually bites in C/C++ implementation".
16. **Unicode identifiers done properly.** `unicode_name_fsm.c` (69 K,
    generated FSM), `dev_tools/cpp_tools/process_unicode_names`,
    `edg-make-confusables-data` (confusable-identifier diagnostics).
17. **Generated-code C output as oracle.** `c_gen_be.c` lets us run
    EDG→C→GCC/Clang as a *semantics oracle* on arbitrary C input — a
    differential-testing arm that needs no EDG codegen.

### Cons (what LCCC must NOT copy)

1. **No optimizer, no codegen.** Zero transplant value for our primary metric
   from the IL→machine path — EDG has none.
2. **Tree IL, not SSA.** EDG's IL is intentionally source-shaped (C++-rich,
   implicit conversions explicit, but no CFG SSA). LCCC's SSA IR + GVN/SCCP is
   already the stronger optimizer substrate. Do **not** regress toward a
   tree IL; port *metadata and evaluation techniques* instead.
3. **Monolithic, global-state-heavy C++ style.** `EXTERN_THREAD` globals
   everywhere (e.g. `processing_file_scope_init_routine`, `ctor_init_this`,
   `pp_if_stack`); `.c` files compiled as C++11; single 1.5 M-line compilation
   unit set. LCCC's module-per-phase Rust is maintainable in ways EDG is not.
4. **Generated files checked in at enormous size** (`builtin_defs.h` 308 K
   lines, `unicode_name_fsm.c` 69 K). LCCC should generate at build time or
   in `build.rs`, not commit such artifacts.
5. **C++-centric complexity dominates**: `templates.c` (47 K), `overload.c`
   (31 K), `class_decl.c` (37 K), IFC-modules stack (~250 K lines of generated
   `ifc_map_functions*.c`) — irrelevant for LCCC's C mission. Skip wholesale.
6. **CMake/Docker-oriented dev flow** with hard-coded host paths in the
   builtins scraper ("will not work out-of-the-box for you" — their words).
   The *ideas* are portable; the tooling needs rewriting against our
   fastbuild/ci_local conventions.
7. **Interpretation is C++-constexpr-shaped.** Its cost model
   (2·calls+loop-backs) targets C++ constexpr rules; C23 constexpr is smaller
   but our `_Static_assert`/static-initializer evaluator must additionally
   model C's unsequenced/UB rules (see their `N1282` unsequenced-modification
   test in `imported/clang/c`).
8. **`--prelink` template instantiation model** is a legacy solution to a
   problem C doesn't have.

---

## 4. Subsystem-by-subsystem transplant plan

Each item: **Evidence → Gap in LCCC → Action**. Tiers: **T0** = do now
(hours, high confidence), **T1** = next sessions (days), **T2** = research
spikes (prototype + measure), **T3** = explicitly rejected (recorded so future
agents don't re-litigate).

### 4.1 Frontend: preprocessing & lexing

**Evidence.** `$EDG/src/lexical.c` (31 K), `preproc.c`, `macro.c`,
`doc/source/lex_pp.rst` (2 138 lines) — token-preserving expansion with
modification lists; `ATTENTION_MARKER` scheme; comment/trigraph/line-splice
reconstruction for accurate columns; inert-macro marking; `LE_END_OF_TOKEN`
separators in macro text.

**LCCC gap.** `src/frontend/preprocessor/` (8 338 lines) is a **text→text**
pipeline producing an expanded `String` with `# line` markers. It is fast and
simple but (a) loses token provenance inside macro expansions (diagnostics
point at expanded text), (b) re-lexes the expanded string (double work),
(c) cannot implement `-E` with comment retention or `__has_include`-style
position-sensitive features as precisely.

**Action (T1).** Do *not* rewrite the preprocessor wholesale. Extract two
designs:
1. **Modification-list diagnostics**: when lccc reports a diagnostic in
   macro-expanded code, attach a "in expansion of macro X defined here"
   note chain. This is the single most-used EDG preprocessor feature in
   practice and needs only per-token origin tracking in the lexer, not a new
   preprocessor. Track via a side-table from expanded offsets → macro
   invocation spans (the line-marker machinery already maps offsets → files).
2. **GCC-dialect switch catalogue**: harvest `cmd_line.c`'s GNU/MSVC
   emulation flag list into `docs/` as the compatibility checklist for
   lccc's `--gnu`-style flags (`emulate_gnu_abi_bugs` class of behaviors:
   zero-width bit-field alignment, `-fms-extensions` vs `-fms-compatibility`
   split, `__thread` defaults, etc.).

### 4.2 Frontend: parsing/sema edge knowledge

**Evidence.** `imported/clang/c/{Parser,Sema,Preprocessor,C}/` — 1 529 Clang
C tests in `.sft.c` form (Apache-2.0+LLVM-exception), covering N-documents
(WG14 papers), unsequenced modifications (N1282), etc. `tests/edg/*` — 9 006
EDG-authored tests. `src/Changes` — every edge case with a bug ID.

**LCCC gap.** `tests/regression/` is hand-grown; no mined external corpus
beyond what `x86_gcc_torture.py` drives against a GCC checkout.

**Action (T0).** `scripts/edg_corpus_mine.py` (landed with this session):
license-aware extractor that
- copies/adapts `imported/clang/c/**/*.sft.c` (Apache) into
  `tests/corpus/clang-c/` in lccc's regression format (expect gcc-identical
  stdout/exit via the existing A/B differential harness);
- for `imported/gnu/c/*.sft.c` (**GPL-3 — never copied into the repo**),
  records only the GCC testcase *names* (e.g. `20000108-1`) in a manifest so
  `scripts/x86_gcc_torture.py`-style drivers can fetch the originals from a
  user-local GCC checkout at run time;
- flags every test with `//match_regex`/`//type` metadata for triage.

**Action (T1).** Mine `src/Changes` (C-relevant entries: C23 tag compatibility
N3037, empty initializers, bit-field quirks, `__auto_type`, statement
expressions) into `tests/bugs/` regression tests for lccc. One entry = one
reproducer. This is the highest-density correctness oracle in the whole EDG
repo.

### 4.3 Diagnostics infrastructure

**Evidence.** `$EDG/src/error_msg.txt` (10 449 lines), `error_tag.txt`,
`util/mk_errinfo.c` → generated `err_codes.h`/`err_data.h`;
`doc/source/err_msgs.rst` auto-generated; severity-override tags on the
command line; multi-message diagnostics with indentation rules
(`error.c` `LIST_DIAG_INDENT`, catastrophe-loop guard).

**LCCC gap.** `src/common/error.rs` (961 lines) has structured
`Diagnostic{severity, location, message}` with color support, but messages are
inline strings: no stable codes, no tags, no central catalog, no generated
docs, no `-Wno-*`/`-Werror=*` per-diagnostic control.

**Action (T1).** Adopt the *shape* (not the code). New files to create
(proposed layout):

```text
src/diagnostics/messages.toml     single source of truth: code, tag, text,
                                  default-severity, since
scripts/gen_diag_catalog.py       generator (or build.rs) emitting the Rust
                                  enum + static table + docs/diagnostics.md
```

1. `messages.toml` (or a `.rs` table) as single source of truth.
2. Small generator emitting the Rust enum + static table + the generated
   diagnostics reference page.
3. CLI: `-Werror=<tag>`, `-Wno-<tag>`, `--print-diagnostic-tags`.
   This directly serves the oracle workflow ("explain why this warning
   fired/not") and C-dialect compatibility testing.

### 4.4 Constant evaluation & arithmetic

**Evidence.** `interpret.c` (34.8 K) — work-stack interpreter, cost budget,
4 storage pools, pointer-validity, `interpret_dynamic_init`,
`is_core_constant_expr` + diagnostic lists; `folding.c` (overflow severity
policies per dialect/mode, complex folding, fixed-point); `const_ints.c`
(multi-word target integers); `floating.c` (Steele-White/Clinger/Gay exact
conversions); `float_pt.c`/`fixed_pt.c`.

**LCCC gap.** `common/const_eval.rs` (437) + `const_arith.rs` (889) +
`frontend/sema/const_eval.rs`: literal/builtin folding shared between sema and
lowering via closures. Deep-expression recursion (no work-stack), no cost
budget, no explicit storage model (fine for C today), decimal handling in
`common/decimal.rs` + `check_decimal_const_agrees_with_gcc.sh` already
exists — so exactness is partly there.

**Action (T1).**
1. **Work-stack evaluator**: convert `const_eval` recursion to EDG-style
   explicit stack + step budget. Direct payoff: `#if` expressions and static
   initializers in adversarial/generated input can no longer blow the Rust
   stack (we have `crash_synth_*` repros in `artifacts/repros/` — deep
   synthetic expressions are a known crash class). Differential-fuzz the new
   evaluator against GCC's `__builtin_constant_p` behavior.
2. **Cost/step budget for const-eval** with a clean diagnostic
   ("evaluation exceeded budget") instead of OOM/hang — matches EDG's
   termination reason (3).
3. Port the **overflow-severity policy matrix** (`ES_INT_OVERFLOW` selection
   by dialect: constexpr vs strict-ANSI vs GNU) as the model for lccc's
   constant-overflow diagnostics in `-std=` modes.
4. Keep i64/i128 fast paths (LCCC's strength) but adopt `const_ints.c`'s
   part-array generalization *behind the same API* for `_BitInt`/extreme
   widths later (T2).

### 4.5 ABI layout (bit-fields & friends)

**Evidence.** `layout.c` + `targ_def.h` (5 920 lines of target knobs):
MS bit-field containers, negative container-size GNU mode, zero-width
alignment quirks gated on `emulate_gnu_abi_bugs`, empty-base optimization,
field-order policies, `[[no_unique_address]]`.

**LCCC gap.** C struct layout is implemented in
`ir/lowering/structs.rs` (2 205) + `common/types.rs`; bit-field lowering has
had repeated fixes (see `engineering/FOLLOWUP-*` history). Dialect-specific
bit-field allocation strategies are the classic "GCC vs Clang vs MSVC"
compatibility minefield — for the Linux kernel/glibc corpus, **GNU bit-field
quirks are load-bearing**.

**Action (T1).** Build a **bit-field ABI differential corpus**: enumerate
(container type, width sequence, signedness, alignment) patterns, compile
with lccc + GCC + Clang, compare `offsetof`/`sizeof` dumps; encode the
resulting rules table as unit tests (`tests/correctness/bitfield_abi/`).
EDG's `layout.c` decision tree is the checklist of *which* variants exist
(MS container rules, negative-container GNU mode, zero-width rules). This is
pure correctness insurance for kernel/glibc builds and needs no EDG code
copying (algorithm descriptions in `layout.c` comments are Apache-licensed
guidance).

### 4.6 Builtins

**Evidence.** `dev_tools/builtins/`: `extract-builtin-declarations` +
`print-gcc-builtins.cpp`/`print-clang-builtins.cpp` **scrape signatures from
installed compiler binaries via their plugin/debug interfaces** (explicitly
"to avoid any legal issues"); `generate-builtin-table.py` merges
`builtins_{Lg}x_<version>_{a,m}{32,64}.txt` snapshots → `builtin_defs.h` +
`builtin_kinds.h`.

**LCCC gap.** `frontend/sema/builtins.rs` (3 601) is a hand-written
name→libc map (`__builtin_memcpy`→`memcpy`, …) plus direct handling of a few
(`__builtin_trap`). No signature database → no arity/type checking of builtin
calls, no per-target `__builtin_cpu_*` model, no systematic coverage of the
~1 500 GCC/Clang builtins (the kernel alone uses hundreds of
`__builtin_*`/`__sync_*`/`__atomic_*`).

**Action (T1).** Port the *pipeline*:
1. `scripts/scrape_builtins_{gcc,clang}.py` — drive `gcc -Q --help=target`? No:
   better, EDG's approach is compile a generated `.c` that asks the compiler
   to dump builtin decls (GCC: plugin headers; Clang: `__clang__` +
   `Builtin::Context` via `clang -cc1 -ast-dump` on a file using
   `__has_builtin`). For lccc, a portable variant: generate probes
   `void probe(void){ (void)__builtin_foo; }` and parse `gcc/clang -fsyntax-only`
   acceptance + a small `#pragma GCC diagnostic` harness to recover arity via
   deliberate wrong-arity calls. Store as versioned
   `data/builtins/{gcc-16.2,clang-23.1}.json`.
2. Generate a Rust table (build.rs) with name, signature, target guards,
   const-foldability, and lowering target (libc call / IR intrinsic /
   unsupported-diagnostic).
3. **Win condition**: Linux kernel + glibc + zlib-ng/expat compile with
   zero unresolved `__builtin_*` and correct prototype checking; wrong-arity
   builtin calls now diagnose instead of miscompile.

### 4.7 Tests & methodology

**Evidence.** Testing framework (`doc/source/testing.rst`): `.sft`
directives, `TEST_NUMBER` variants, `--record` mode, per-target expectation
directories, output normalization, `edg-test-run-sonar` (flaky-test radar),
`edg-bisect`, `edgy-reduce-race` (parallelism-flake reducer),
`edg-walk-tests`, `edg-examine-test`, `clone-test-here`. Benchmarks:
Cachegrind fixed geometry + baselines + delta runs + review thresholds.

**LCCC gap.** `ci_local.sh` is excellent as a gate, and
`run_regression_suite.sh` does lccc-vs-GCC + A/B differential; but there is
no (a) expectation re-recording workflow, (b) flake sonar, (c) test-case
reducer beyond Csmith tooling, (d) deterministic cross-host cache-simulation
benchmark tier (we have `callgrind_ab.py` — **pinned this session**, see
E2 status in §6).

**Action (T0/T1).**
1. **T0 (done 2026-10-01):** `scripts/callgrind_ab.py` now pins Cachegrind
   geometry `--I1/--D1/--LL` to EDG's fixed geometry
   (32768,8,64 / 32768,8,64 / 33554432,16,64) so VM measurements are
   comparable across sessions/hosts (*the brilliant no-PMU method:
   instruction counts and simulated misses are bit-deterministic*). Sweep
   the remaining A/B scripts for Valgrind use in E13 if any appear.
2. **T1:** an expect-recording script (`expect_record`, E13) —
   golden-output re-recording for `tests/regression/` with explicit diff
   review (mirror `edg-run-test --record`).
3. **T1:** flake sonar: run the unit suite N times
   (`LCCC_TEST_REPEATS` already exists in ci_local) and statistically flag
   order-dependent tests (mirror `edg-test-run-sonar`).
4. **T2:** `creduce`-style test reduction scripted against lccc crashes
   (we already keep `artifacts/repros/crash_*`; formalize the loop).

### 4.8 IL / IR design lessons

**Evidence.** `il_def.h` (19.5 K) + `il.rst` (3 562): entities carry source
ranges + source-sequence entries; shareable constants interned; region-based
allocation; `il_walk.c` generic walkers; `il_read`/`il_write` versioned file
format; `il_to_str`/`il_display` dumps; `MAINTAIN_NEEDED_FLAGS`-style lazy
maintenance flags documented per entity.

**LCCC gap.** SSA IR is the right optimizer substrate (keep!), but: alias/TBAA
metadata propagation (charter §14/§16), range info, and IR
serialization/diffing for regression forensics are the known gaps.

**Action (T2, charter-aligned).**
1. **IR file format + textual dump + structural diff** (`--emit-ir`,
   `--ir-diff`): EDG's `il_write/il_read/il_display` is the model. Payoff:
   pass-by-pass regression bisection ("which pass changed this IR?") and
   godbolt-independent evidence artifacts.
2. **Shareable-constant interning** for LCCC's `IrConst` (hash+eq table) —
   cheap memory win in large TUs (kernel!).
3. Keep per-entity `needed`/`dirty` flags (EDG `MAINTAIN_NEEDED_FLAGS`)
   discipline when adding IR metadata so analyses can stay incremental.

### 4.9 Lowering knowledge (C-specific)

**Evidence.** `lower_c99.c` (C99→C89 lowering incl. VLAs, GNU extensions),
`lower_init.c` (20.6 K: initializer lowering with pending-stmt lists,
implied-copy sources, subobject `this` stacks), `decl_inits.c` (initializer
scanning), `lower_eh.c`, `lower_name.c` (ABI name mangling — the *IA-64
C++ ABI* implementation lives here).

**LCCC gap.** `ir/lowering/global_init.rs` (2 308) + `stmt.rs`/`structs.rs`
handle C initializers; VLAs and nested/designated initializer corner cases are
perennially bug-prone (see bugs corpus).

**Action (T1).**
1. Build `tests/correctness/initializers/` mined from
   `imported/gnu/c` *names* (fetch originals from local GCC checkout) +
   Clang corpus (Apache): designated initializers, nested braces, string
   literal init, flexible array members, compound literals, `_Static_assert`
   interactions.
2. VLA lowering differential tests (EDG `lower_c99.c`'s VLA section is the
   checklist: scope-exit cleanup, `sizeof` re-evaluation, compound-literal
   lifetimes).

### 4.10 Compile-time performance & driver

**Evidence.** `pch.c` (precompiled headers on IL regions), `cfe_daemon.c`
(FE-as-daemon over Unix socket, thread-per-client),
`cfe_daemon_common.h` protocol, `dev_tools` CE packer.

**LCCC gap.** No PCH, no daemon. Startup + repeated-header parse dominates
kernel/glibc builds.

**Action (T2).** For the fast-react-loop mission the highest-value EDG idea is
the **daemon**: a long-lived lccc server holding parsed headers (or even
pre-lowered IR for system headers) behind a small protocol; the CLI becomes a
thin client. Prototype behind `lccc --daemon` with a socket in `$XDG_RUNTIME_DIR`;
measure kernel-subsystem compile latency. PCH (serialized preprocessor state +
AST for headers) is the fallback if the daemon is too invasive.

### 4.11 Unicode identifiers

**Evidence.** `unicode_name_fsm.c` (generated), confusables data tooling.

**LCCC gap.** C23 allows Unicode identifiers in some modes; kernel/glibc
don't need it. **T3 (defer)** — but keep the *generated-FSM* technique in
mind if we ever add it; also `edg-make-confusables-data`'s "warn on visually
ambiguous identifiers" is a cheap, delightful diagnostic (optional T3).

### 4.12 Rejected transplants (recorded so we don't re-litigate)

| Item | Why rejected |
|---|---|
| Template machinery (`templates.c`, prelinker, IFC modules) | C-only mission; 300 K+ lines of irrelevance |
| Tree IL as primary IR | Would forfeit SSA-based optimization quality |
| C++-generating backend (`cp_gen_be.c`) | Source-to-source is not our product; `c_gen_be` usable only as oracle |
| MS metadata (`ms_metadata.cpp`), C++/CLI | Out of scope |
| `disambig.c` C++ ambiguity hacks | C has no such ambiguity |
| Committing generated 300 K-line headers | Build-time generation instead |
| `cfe_daemon` code as-is | Exposition-grade per its own README; port the *design* only |

---

## 5. Licensing constraints (hard law for the transplant)

| Tree | License | Rule for LCCC |
|---|---|---|
| EDG core (`src/`, `doc/`, `dev_tools/`) | Apache-2.0 WITH LLVM-exception | Code/ideas usable; keep SPDX attribution on copied files; Apache obligations apply |
| `tests/tests/imported/clang/**` | Apache-2.0 WITH LLVM-exception | **Copy/adaptable** into lccc tests with attribution headers |
| `tests/tests/imported/gnu/**` | **GPL-3.0** | **NEVER copy into lccc.** Reference by testcase name; fetch originals from a local GCC checkout at test time; or reimplement behavior from the standard/bug reports |
| `tests/expectations/**` (recorded outputs of the above) | follows the underlying test | same rule as the test |
| Ideas/algorithms described in docs/comments | Apache-2.0 | implement clean-room in Rust with a doc comment crediting the design source |

`third_party_licenses/` in lccc must gain a `EDG-NOTICE` entry if any
Apache-2.0+LLVM-exception file is copied verbatim.

---

## 6. Prioritized action register

| ID | Item | Tier | Effort | Expected gain | Workload impact |
|---|---|---|---|---|---|
| E1 | `scripts/edg_corpus_mine.py` + Clang-C corpus import | T0 | S | correctness net (1 529 tests) | all (frontend) |
| E2 | Pin Cachegrind geometry in A/B scripts (fixed I1/D1/LL) | T0 | S | reproducible no-PMU perf data | all (measurement) |
| E3 | Builtin signature scrape+generate pipeline | T1 | M | kernel/glibc/expat compat; fewer miscompiles | kernel, glibc, zlib-ng, expat |
| E4 | Diagnostic catalog + tags + `-Werror=` | T1 | M | explainability, dialect testing | all |
| E5 | Work-stack const-eval + step budget | T1 | M | crash-class elimination (`crash_synth_*`) | fuzz corpus, kernel |
| E6 | Bit-field ABI differential corpus | T1 | M | ABI exactness | kernel, glibc |
| E7 | `Changes`-miner → C regression tests | T1 | L | decades of edge cases | all |
| E8 | Macro-expansion diagnostic notes | T1 | S | usability, debugging speed | all |
| E9 | Initializer/VLA differential corpus | T1 | M | correctness | kernel, glibc |
| E10 | IR dump/diff tooling | T2 | M | pass bisection, evidence artifacts | optimizer dev |
| E11 | lccc daemon (FE-as-service) | T2 | L | compile latency | kernel builds |
| E12 | Shareable-const interning | T2 | S | peak RSS on big TUs | kernel |
| E13 | Flake sonar + expect-recording | T1 | S | CI stability | dev loop |
| E14 | `-E` comment-retention / provenance output | T3 | — | only tooling consumers | — |

**Status 2026-10-01 (this session):** **E1 landed** (`scripts/edg_corpus_mine.py`
selftest 18/18 + `tests/corpus/`: 1 434 Apache-licensed Clang-C tests mined
with SPDX attribution, metadata consolidated in `clang-c/corpus-index.json`
(expectations with source lines, per-invocation `option_sets`, EDG→GCC flag
translation), 95 giants capped with reasons, 18 188 GPL-safe GNU test names
manifested as JSONL, runner `tests/corpus/run_clang_c_corpus.py`).
**E2 landed** (`scripts/callgrind_ab.py` pins `--I1=32768,8,64 --D1=32768,8,64
--LL=33554432,16,64`, env-overridable via `LCCC_CG_{I1,D1,LL}`, geometry and
compiler versions recorded in `manifest.json`; the current audit used mocked
Callgrind orchestration only, not native Valgrind/performance evidence).
**E7 accelerated** (`scripts/edg_changes_mine.py` dual-era parser: **13 562**
`src/Changes` entries parsed losslessly (7 416 bracketed + ~6 146 pre-2008
plain headers, wrap-aware), 4 upstream date typos adjudicated
(`127`→2017, `37`→1997, `69`→1996, `82`→1992), **1 559** C-relevant entries
extracted to `docs/edg_changes_c_extract.md` (914 KB, no mega-blob) + full
index + stats — turning E7 from "mine manually" into "curate the extract into
regression tests"). All three were re-opened and perfected against the PR
#719 review audit (F1–F8, adjudicated in `docs/REVIEW_719_ADJUDICATION.md`).
E3–E6, E8–E14 open.

**Sequencing for the codegen mission:** E2 first (measurement is the
bottleneck of every claim), then E1/E5 (correctness floor), then E3/E6/E9
(workload compile fidelity → measurable perf runs on zlib-ng/gzip/expat),
then E7 (continuous hardening), E10+ as the optimizer research accelerators.

---

## 7. Verification of this analysis

Facts in this document were taken from the cloned tree on 2026-10-01:

- LOC counts: `wc -l src/*.c src/*.h` (1 573 339),
  `find src -name '*.rs' | xargs wc -l` in lccc (674 208).
- Test counts: `find tests/tests -name '*.sft.c'` (19 717) /
  `'*sft.cpp'` (25 392) = 45 109 single-file tests; EDG-authored `.sft`
  tests 2 925 (9 006 files incl. recorded outputs); imported `.sft` tests
  40 018 (95 752 files incl. outputs); Clang-C subtree 1 529 `.sft.c`;
  GNU-C subtree 18 188 `.sft.c`.
- Cachegrind geometry literal: `dev_tools/bin/edg-bench` lines 28–34.
- Builtins pipeline: `dev_tools/builtins/README.txt`.
- Interpreter design: `$EDG/src/interpret.c` header comment (work stack,
  cost model, storage pools).
- Diagnostic catalog: `$EDG/src/error_msg.txt` header comment +
  `util/mk_errinfo.c`.
- License headers: `LICENSE.txt`, `tests/tests/imported/{gnu,clang}/LICENSE.txt`
  (spot-checked: no GPL material outside `imported/gnu` and one vendored
  `edggpp/LICENSE.txt` in dev_tools; every `src/*.c` carries the
  Apache-2.0+LLVM-exception SPDX).

**Harness-wipe note (important for future agents):** mid-session the
execution harness truncated the 3.7 GB EDG clone to 104 MB and deleted
lccc's `.git` (the documented 10 k-file/128 MB snapshot cap). All counts
above were re-verified against a fresh clone *before* the extracts were
regenerated; the three knowledge artifacts under `tests/corpus/` and
`docs/edg_changes_*` are re-derivable in ~1 minute from any EDG clone via
the two mining scripts, and the mining scripts are self-contained. Do not
trust any EDG-tree file count older than the last full clone — re-clone
(`git clone --filter=blob:none --depth 1`) and re-verify.
