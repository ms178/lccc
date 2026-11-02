# C23 accept/reject inventory vs GCC 16.2

Measured: 2026-10-27<br>
Oracle: GCC 16.2.0 (`gcc-16`)<br>
Compiler: `target/fastbuild/lccc`<br>
Probes: 24 translation units. This is not an overall C23 coverage percentage.

Every probe is compiled with `-fsyntax-only` in the listed mode. C17 outcomes are measured and recorded; a C17 acceptance may be a GNU extension, so it is not inferred from the standard.

| Feature / probe | GCC C23 | LCCC C23 | GCC C17 | LCCC C17 | Expected GCC C23 |
|---|---:|---:|---:|---:|---:|
| **default-gnu23-version** — default mode defines the GCC 16.2 C23 __STDC_VERSION__ value | ACCEPT | ACCEPT | REJECT | REJECT | ACCEPT |
| ↳ default-mode control | ACCEPT | ACCEPT | — | — | ACCEPT |
| **bool-true-false** — C23 bool, true, and false spellings | ACCEPT | ACCEPT | REJECT | REJECT | ACCEPT |
| **nullptr** — nullptr converts to object pointers and compares with nullptr | ACCEPT | ACCEPT | REJECT | REJECT | ACCEPT |
| **constexpr-named-constant** — constexpr object usable by a static assertion and array bound | ACCEPT | ACCEPT | REJECT | REJECT | ACCEPT |
| **auto-type-inference** — C23 auto infers an object type from its initializer | ACCEPT | ACCEPT | REJECT | ACCEPT | ACCEPT |
| **typeof-and-typeof-unqual** — typeof preserves qualification and typeof_unqual removes it | ACCEPT | ACCEPT | REJECT | REJECT | ACCEPT |
| **fixed-enum-underlying-type** — C23 fixed underlying enum type and its size | ACCEPT | ACCEPT | ACCEPT | ACCEPT | ACCEPT |
| **digit-separator** — digit separator in an integer constant | ACCEPT | ACCEPT | REJECT | REJECT | ACCEPT |
| **empty-initializer** — empty initializer braces zero-initialize an aggregate | ACCEPT | ACCEPT | ACCEPT | ACCEPT | ACCEPT |
| **static-assert-keyword** — static_assert keyword form | ACCEPT | ACCEPT | REJECT | ACCEPT | ACCEPT |
| **alignof-keyword** — alignof keyword and aligned declaration | ACCEPT | ACCEPT | REJECT | ACCEPT | ACCEPT |
| **standard-maybe-unused-attribute** — C23 [[maybe_unused]] attribute on a block-scope object | ACCEPT | ACCEPT | ACCEPT | ACCEPT | ACCEPT |
| **unnamed-variadic-function** — C23 function definition with an unnamed variadic parameter list | ACCEPT | ACCEPT | ACCEPT | ACCEPT | ACCEPT |
| **one-argument-va-start** — C23 one-argument va_start form from the implementation stdarg.h | ACCEPT | ACCEPT | REJECT | ACCEPT | ACCEPT |
| **elifdef** — #elifdef selects a branch when the macro is defined | ACCEPT | ACCEPT | REJECT | ACCEPT | ACCEPT |
| **elifndef** — #elifndef selects a branch when the macro is not defined | ACCEPT | ACCEPT | REJECT | ACCEPT | ACCEPT |
| **embed-bytes** — #embed contributes resource bytes to an initializer | ACCEPT | ACCEPT | ACCEPT | ACCEPT | ACCEPT |
| **has-embed-operator** — __has_embed reports a present resource | ACCEPT | ACCEPT | ACCEPT | ACCEPT | ACCEPT |
| **va-opt** — __VA_OPT__ conditionally emits variadic macro tokens | ACCEPT | ACCEPT | ACCEPT | ACCEPT | ACCEPT |
| **bitint-17** — C23 _BitInt(17) integer type (included as a coverage gap probe) | ACCEPT | REJECT | REJECT | REJECT | ACCEPT |
| **float16-gcc-extension** — GCC _Float16 extension in the C23 dialect | ACCEPT | ACCEPT | ACCEPT | ACCEPT | ACCEPT |
| **nullptr-arithmetic-rejected** — invalid arithmetic on nullptr is rejected in C23 mode | REJECT | REJECT | REJECT | REJECT | REJECT |
| **empty-list-prototype-arity** — C23 f() is a zero-parameter prototype; C17 f() remains unspecified | REJECT | REJECT | ACCEPT | ACCEPT | REJECT |
| **empty-list-prototype-valid-call** — an empty-list prototype accepts a zero-argument call in both modes | ACCEPT | ACCEPT | ACCEPT | ACCEPT | ACCEPT |

## Measured C23 differences

- `bitint-17`: GCC 16.2 accepts `unsigned _BitInt(17)`; LCCC rejects it in C23
  and C17. This is a deferred implementation gap, not an intentional C23
  rejection. Do not add syntax-only acceptance or change the oracle expectation:
  the fixed-width `CType` model cannot preserve arbitrary precision and
  signedness through integer conversions, constant evaluation, object and
  aggregate layout, ABI classification, code generation, and debug metadata.
  Keep this row visible until end-to-end support has compiler-oracle, ABI, and
  layout coverage; see the live `_BitInt` item in `backlog.md`.

## Measured C17 mode differences

LCCC accepts these probes in `-std=c17`, while GCC 16.2 rejects them in that
mode: `auto-type-inference`, `static-assert-keyword`, `alignof-keyword`,
`one-argument-va-start`, `elifdef`, and `elifndef`. These are LCCC C17-mode
extensions for this probe set, not C23-mode mismatches. GCC also accepts several
C23-era spellings as GNU extensions in C17 (shown separately in the table).

## Method and scope

The report was refreshed on 2026-10-27 with `tests/regression/check_c23_inventory.py`
against GCC 16.2.0 and the current fastbuild compiler. Each probe is compiled
with `-fsyntax-only`; GCC's declared C23 expectations are checked, and C23 and
C17 LCCC/GCC verdict differences are reported separately. The machine-readable
run preserves each probe's source hash, full diagnostics, and argv; it is
produced with the same script's `--json` option and kept as task evidence outside
the repository rather than committed as generated data.

This is a small per-feature accept/reject sample, not a comprehensive C23
conformance suite and not a coverage percentage. It does not establish runtime,
ABI, layout, code-generation, or exhaustive half-format correctness. In
particular, the `_Float16` probe is a single syntax/semantic sample, not an
exhaustive check across all 2^16 encodings and rounding modes.
