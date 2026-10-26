# C23 accept/reject inventory vs GCC 16.2

Measured: 2026-10-26<br>
Oracle: GCC 16.2.0 (`/home/user/toolchains/gcc16.2/prefix/bin/gcc-16`)<br>
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

- `bitint-17`

Source hashes, complete diagnostics, and argv are preserved when the same run is requested with `--json`.
