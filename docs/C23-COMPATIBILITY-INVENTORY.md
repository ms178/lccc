# C17/C23 dialect inventory vs GCC 16.2

Measured: 2026-11-04<br>
Oracle: GCC 16.2.0 (`gcc-16`)<br>
Compiler: `target/fastbuild/lccc`<br>
Probes: 43 translation units. This is not an overall C23 coverage percentage.

Each probe is compiled with `-fsyntax-only` in every dialect below; a cell reads `GCC/LCCC` and a pair that disagrees is a failure unless the probe is a declared deviation. Probes that do not name an expectation for a GNU dialect inherit the expectation of the strict dialect it extends.

| Feature / probe | C23 | GNU23 | C17 | GNU17 |
|---|---:|---:|---:|---:|
| **default-gnu23-version** — default mode defines the GCC 16.2 C23 __STDC_VERSION__ value | ACCEPT/ACCEPT | — | REJECT/REJECT | — |
| **bool-true-false** — C23 bool, true, and false spellings | ACCEPT/ACCEPT | ACCEPT/ACCEPT | REJECT/REJECT | REJECT/REJECT |
| **nullptr** — nullptr converts to object pointers and compares with nullptr | ACCEPT/ACCEPT | ACCEPT/ACCEPT | REJECT/REJECT | REJECT/REJECT |
| **constexpr-named-constant** — constexpr object usable by a static assertion and array bound | ACCEPT/ACCEPT | ACCEPT/ACCEPT | REJECT/REJECT | REJECT/REJECT |
| **auto-type-inference** — C23 auto infers an object type from its initializer | ACCEPT/ACCEPT | ACCEPT/ACCEPT | REJECT/REJECT | REJECT/REJECT |
| **typeof-and-typeof-unqual** — typeof preserves qualification and typeof_unqual removes it | ACCEPT/ACCEPT | ACCEPT/ACCEPT | REJECT/REJECT | REJECT/REJECT |
| **fixed-enum-underlying-type** — C23 fixed underlying enum type and its size | ACCEPT/ACCEPT | ACCEPT/ACCEPT | ACCEPT/ACCEPT | ACCEPT/ACCEPT |
| **digit-separator** — digit separator in an integer constant | ACCEPT/ACCEPT | ACCEPT/ACCEPT | REJECT/REJECT | REJECT/REJECT |
| **empty-initializer** — empty initializer braces zero-initialize an aggregate | ACCEPT/ACCEPT | ACCEPT/ACCEPT | ACCEPT/ACCEPT | ACCEPT/ACCEPT |
| **static-assert-keyword** — static_assert keyword form | ACCEPT/ACCEPT | ACCEPT/ACCEPT | REJECT/REJECT | REJECT/REJECT |
| **alignof-keyword** — alignof keyword and aligned declaration | ACCEPT/ACCEPT | ACCEPT/ACCEPT | REJECT/REJECT | REJECT/REJECT |
| **standard-maybe-unused-attribute** — C23 [[maybe_unused]] attribute on a block-scope object | ACCEPT/ACCEPT | ACCEPT/ACCEPT | ACCEPT/ACCEPT | ACCEPT/ACCEPT |
| **unnamed-variadic-function** — C23 function definition with an unnamed variadic parameter list | ACCEPT/ACCEPT | ACCEPT/ACCEPT | ACCEPT/ACCEPT | ACCEPT/ACCEPT |
| **one-argument-va-start** — C23 one-argument va_start form from the implementation stdarg.h | ACCEPT/ACCEPT | ACCEPT/ACCEPT | REJECT/REJECT | REJECT/REJECT |
| **elifdef** — #elifdef selects a branch when the macro is defined | ACCEPT/ACCEPT | ACCEPT/ACCEPT | REJECT/REJECT | ACCEPT/ACCEPT |
| **elifndef** — #elifndef selects a branch when the macro is not defined | ACCEPT/ACCEPT | ACCEPT/ACCEPT | REJECT/REJECT | ACCEPT/ACCEPT |
| **embed-bytes** — #embed contributes resource bytes to an initializer | ACCEPT/ACCEPT | ACCEPT/ACCEPT | ACCEPT/ACCEPT | ACCEPT/ACCEPT |
| **has-embed-operator** — __has_embed reports a present resource | ACCEPT/ACCEPT | ACCEPT/ACCEPT | ACCEPT/ACCEPT | ACCEPT/ACCEPT |
| **va-opt** — __VA_OPT__ conditionally emits variadic macro tokens | ACCEPT/ACCEPT | ACCEPT/ACCEPT | ACCEPT/ACCEPT | ACCEPT/ACCEPT |
| **bitint-17** — C23 _BitInt(17) integer type (included as a coverage gap probe) | ACCEPT/REJECT | ACCEPT/REJECT | REJECT/REJECT | REJECT/REJECT |
| **float16-gcc-extension** — GCC _Float16 extension in the C23 dialect | ACCEPT/ACCEPT | ACCEPT/ACCEPT | ACCEPT/ACCEPT | ACCEPT/ACCEPT |
| **nullptr-arithmetic-rejected** — invalid arithmetic on nullptr is rejected in C23 mode | REJECT/REJECT | REJECT/REJECT | REJECT/REJECT | REJECT/REJECT |
| **empty-list-prototype-arity** — C23 f() is a zero-parameter prototype; C17 f() remains unspecified | REJECT/REJECT | REJECT/REJECT | ACCEPT/ACCEPT | ACCEPT/ACCEPT |
| **empty-list-prototype-valid-call** — an empty-list prototype accepts a zero-argument call in both modes | ACCEPT/ACCEPT | ACCEPT/ACCEPT | ACCEPT/ACCEPT | ACCEPT/ACCEPT |
| **implicit-int-storage-class** — a declaration with a storage class and no type is implicit int (C89 only) | REJECT/REJECT | REJECT/REJECT | REJECT/REJECT | REJECT/REJECT |
| **implicit-int-auto-c17** — `auto x = 3;` is C23 inference, and implicit int before it | ACCEPT/ACCEPT | ACCEPT/ACCEPT | REJECT/REJECT | REJECT/REJECT |
| **static-assert-identifier-c17** — `static_assert` is an ordinary identifier outside C23 | REJECT/REJECT | REJECT/REJECT | ACCEPT/ACCEPT | ACCEPT/ACCEPT |
| **alignof-identifier-c17** — `alignof` is an ordinary identifier outside C23 | REJECT/REJECT | REJECT/REJECT | ACCEPT/ACCEPT | ACCEPT/ACCEPT |
| **alignas-identifier-c17** — `alignas` is an ordinary identifier outside C23 | REJECT/REJECT | REJECT/REJECT | ACCEPT/ACCEPT | ACCEPT/ACCEPT |
| **u8-identifier** — `u8` stays an ordinary identifier in every dialect: it is only a prefix when a string or character constant follows | ACCEPT/ACCEPT | ACCEPT/ACCEPT | ACCEPT/ACCEPT | ACCEPT/ACCEPT |
| **u8-char-literal** — `u8'x'` is a C23 character constant, not an identifier before it | ACCEPT/ACCEPT | ACCEPT/ACCEPT | REJECT/REJECT | REJECT/REJECT |
| **stdc-version-c17** — the C17 dialects define __STDC_VERSION__ 201710L and nothing newer | REJECT/REJECT | REJECT/REJECT | ACCEPT/ACCEPT | ACCEPT/ACCEPT |
| **stdc-version-c23** — the C23 dialects define __STDC_VERSION__ 202311L | ACCEPT/ACCEPT | ACCEPT/ACCEPT | REJECT/REJECT | REJECT/REJECT |
| **strict-ansi-in-iso-modes** — `-std=c17`/`-std=c23` define __STRICT_ANSI__, and a GNU dialect does not | ACCEPT/ACCEPT | REJECT/REJECT | ACCEPT/ACCEPT | REJECT/REJECT |
| **gnu-dialect-rejects-strict-control** — `-std=gnu17`/`-std=gnu23` leave __STRICT_ANSI__ undefined | REJECT/REJECT | ACCEPT/ACCEPT | REJECT/REJECT | ACCEPT/ACCEPT |
| **typeof-unqual-needs-c23** — bare `typeof_unqual` is C23 only; `__typeof_unqual__` is reserved everywhere | ACCEPT/ACCEPT | ACCEPT/ACCEPT | ACCEPT/ACCEPT | ACCEPT/ACCEPT |
| **typeof-unqual-bare-in-gnu17** — bare `typeof_unqual` is not a GNU extension: gcc 16.2 rejects it in gnu17 | ACCEPT/ACCEPT | ACCEPT/ACCEPT | REJECT/REJECT | REJECT/REJECT |
| **macro-arity-too-few** — a short invocation is a constraint violation (6.10.3p4) | REJECT/REJECT | REJECT/REJECT | REJECT/REJECT | REJECT/REJECT |
| **macro-arity-too-many** — a long invocation is a constraint violation (6.10.3p4) | REJECT/REJECT | REJECT/REJECT | REJECT/REJECT | REJECT/REJECT |
| **macro-empty-argument-is-one-argument** — `ONE()` is a valid one-argument invocation of a one-parameter macro | ACCEPT/ACCEPT | ACCEPT/ACCEPT | ACCEPT/ACCEPT | ACCEPT/ACCEPT |
| **macro-variadic-omitted-tail** — the variadic part of a macro argument list may be empty | ACCEPT/ACCEPT | ACCEPT/ACCEPT | ACCEPT/ACCEPT | ACCEPT/ACCEPT |
| **elifdef-in-skipped-group** — before C23 a skipped #elifdef is an unknown directive: the group keeps skipping, so the #else runs instead of the #elifdef branch | REJECT/REJECT | REJECT/REJECT | ACCEPT/ACCEPT | REJECT/REJECT |
| **gnu-extension-accepted-without-pedantic** — GNU extensions stay accepted in strict ISO modes unless -pedantic-errors | ACCEPT/ACCEPT | ACCEPT/ACCEPT | ACCEPT/ACCEPT | ACCEPT/ACCEPT |

## What the matrix pins

Each cell reads `GCC/LCCC` for one probe in one dialect. A pair that disagrees
fails the gate (`tests/regression/check_c23_inventory.py`) unless the probe is
listed in its `KNOWN_DEVIATIONS` ledger -- currently one probe, `bitint-17`, for
the reason given below. That makes this table a regression contract rather than a
snapshot: it is the record of exactly which C23 features work, where the C17
boundary falls, and what the C17-mode extensions are.

Three things about the measurement are deliberate:

* **C17 verdicts are pinned, not merely observed.** GCC's outcome in `-std=c17`
  is recorded per probe in the gate (`C17_VERDICTS`) and asserted on every run,
  so a probe cannot quietly become "unconstrained" and a GCC change shows up as
  a failure instead of a silent drift.
* **A GNU dialect inherits its strict dialect's expectation** unless the probe
  says otherwise. `-std=gnu17` is `-std=c17` plus extensions, so a probe that
  accepts in `c17` must accept in `gnu17`; the probes where GCC's GNU dialect
  differs (`typeof`, `#elifdef`, `#ifndef` control) state their own expectation.
* **A rejected program is not enough.** `tests/regression/check_c_std_diagnostics.py`
  pins *how* each rejection is spelled -- message text and severity against GCC's
  -- plus the `DW_AT_language`/`DW_AT_language_version` pair every dialect puts in
  an object file, because the boundary is only useful if a rejected build says
  why and an accepted build debugs as the revision it claims.

## The dialect boundary, as measured

The capability table below is what `src/common/dialect.rs` encodes and what the
gate re-measures. `A` = accepted, `R` = rejected; LCCC and GCC 16.2 agree on
every row in this table.

| Capability | c89 | c99 | c11 | c17 | c23 | gnu17 | gnu23 |
|---|---:|---:|---:|---:|---:|---:|---:|
| `static_assert`, `alignas`, `alignof` keywords | R | R | R | R | A | R | A |
| `bool`, `true`, `false` keywords | R | R | R | R | A | R | A |
| `thread_local`, `constexpr`, `nullptr` keywords | R | R | R | R | A | R | A |
| `typeof` (bare spelling) | R | R | R | R | A | A | A |
| `typeof_unqual` (bare spelling) | R | R | R | R | A | R | A |
| `asm` (bare spelling) | A | R | R | R | R | A | A |
| `__asm__`, `__typeof__`, `__typeof_unqual__` | A | A | A | A | A | A | A |
| `u8'x'` character constant | R | R | R | R | A | R | A |
| `u8"x"` string literal | R | R | A | A | A | A | A |
| `#elifdef` / `#elifndef` directive | * | * | * | * | A | A | A |
| one-argument `va_start(ap)` | R | R | R | R | A | R | A |

`*` The directive is unknown before C23 in a strict dialect: GCC reports
`invalid preprocessing directive #elifdef` in a group it is compiling and says
nothing in a group it is skipping (where the spelling therefore does *not*
re-open the conditional). LCCC matches both halves, in both positions.

Two rules are worth stating because they are the ones a "C23 keywords are just
GNU extensions" shortcut gets wrong:

* `typeof` **is** a GNU extension as well as a C23 keyword, so `-std=gnu17`
  accepts it; `typeof_unqual` is **not** -- GCC accepts only
  `-std=c23`/`-std=gnu23` for the bare spelling, and the reserved
  `__typeof_unqual__` everywhere.
* `asm` runs the other way: it is a GNU extension that C23 did *not* adopt, so
  `-std=c23` rejects the bare spelling while `-std=gnu23` accepts it.

Everything above is keyed on the *revision* alone where the standard is what
gives the word its meaning (the C23 keywords, `u8'x'`, the one-argument
`va_start`), and on the GNU switch only where the extension is what enables it
(`typeof`, bare `asm`, `#elifdef`). That distinction lives in one type,
`common::dialect::Dialect`, which the lexer, the preprocessor, the parser and the
bundled headers all consult, so a dialect question has one answer rather than one
per call site.

## What the matrix found, and what changed

The table above is the result of measuring first and fixing second. The probes
that used to disagree with GCC, and the change that removed the disagreement:

| Symptom | Cause | Fix |
|---|---|---|
| `static_assert`, `alignas`, `alignof` accepted in `c89`...`gnu17` | keyword table mapped them unconditionally | keyed on `Dialect::keyword_static_assert`/`keyword_alignas`/`keyword_alignof` |
| `u8'x'` accepted in every dialect | the literal-prefix dispatch had no dialect gate | keyed on `Dialect::u8_char_literal`, so pre-C23 lexes `u8` as an identifier |
| `static counter = 5;` and `register y;` accepted in `c17`...`c23` | the storage-class-only fallback returned `int` silently | the parser now reports `type defaults to 'int'` (`-Wimplicit-int`) and the diagnostic engine decides the severity: a warning in `c89`, an error from `c99` on, as GCC does |
| `auto x = 3;` accepted in `c89`...`gnu17` | same fallback, reached before the C23 inference branch | same fix: implicit int before C23, inference in C23 |
| bare `typeof_unqual` accepted in `gnu17` | gated as a GNU extension | keyed on `Dialect::keyword_typeof_unqual` |
| `#elifdef` accepted in strict pre-C23 dialects | handled without a dialect check | a real directive in `gnu*`/C23, an unknown directive before C23, in both the compiling and the skipping position |
| one-argument `va_start(ap)` accepted before C23 | the bundled `<stdarg.h>` defined a variadic macro everywhere | `Dialect::va_start_one_argument`: two parameters before C23 |
| a short or long macro invocation compiled silently | no argument-count check at the invocation site | constraint violation (6.10.3p4) reported with GCC's wording: `macro 'X' requires 2 arguments, but only 1 given` |

The macro-arity item is the one that made the `va_start` bug invisible: with a
missing parameter silently substituted as nothing, `va_start(ap)` expanded to
`__builtin_va_start(ap)` and produced correct code, so the *only* symptom of a
non-conforming program was that GCC would have rejected it.

## Known deviation: `_BitInt`

`bitint-17`: GCC 16.2 accepts `unsigned _BitInt(17)`; LCCC rejects it in every
dialect. This is a deferred implementation gap, not an intentional C23 rejection.
Do not add syntax-only acceptance or change the oracle expectation: the
fixed-width `CType` model cannot preserve arbitrary precision and signedness
through integer conversions, constant evaluation, object and aggregate layout,
ABI classification, code generation, and debug metadata. Keep this row visible
until end-to-end support has compiler-oracle, ABI, and layout coverage; see the
live `_BitInt` item in `backlog.md`.

## Deviations that remain outside the C17/C23 scope

Measured while building the table above, and left as a scoped item rather than
fixed here, because they are all in `-std=c89` and none of them affect `c17`,
`gnu17`, `c23` or `gnu23` (which have no deviations at all):

| Probe | c89 | Note |
|---|---|---|
| `u8"..."` string literal | LCCC accepts, GCC rejects | C11 prefix, not gated on the revision yet |
| `restrict` | LCCC accepts, GCC rejects | C99 keyword, not gated on the revision yet |
| `inline` | LCCC accepts, GCC rejects | C99 keyword, not gated on the revision yet |
| `//` comments | LCCC accepts, GCC rejects | C99 comment form, not gated on the revision yet |

Closing them needs the standard revision to reach the lexer as a number rather
than the single `c23` bit it carries today; the work is tracked in `backlog.md`.

## Method and scope

Refreshed on 2026-11-04 with `tests/regression/check_c23_inventory.py` against
GCC 16.2.0 and Clang 23.1.4. Each probe is compiled with `-fsyntax-only` in every
dialect in the table; GCC's declared expectations are asserted, and every
LCCC-vs-GCC verdict difference fails the gate unless it is a declared deviation.
The machine-readable run (`--json`) preserves each probe's source hash, full
diagnostics and argv. `tests/regression/check_c_std_diagnostics.py` covers the
same boundary from the other side: exact message text and severity, plus the
dialect identity in debug information.

This is a per-feature accept/reject sample, not a comprehensive C23 conformance
suite and not a coverage percentage. It does not establish runtime, ABI, layout,
code-generation, or exhaustive half-format correctness. In particular, the
`_Float16` probe is a single syntax/semantic sample, not an exhaustive check
across all 2\^16 encodings and rounding modes.
