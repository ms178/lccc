# Qualified `typeof`: bounded fixes and open wrong-code cases

The const-qualification fix covers type-name and expression operands,
`typeof_unqual`, pointer typedefs, fields, parameters, nested declarations
and shadowing. Its regression gate checks exact GCC 16.2 diagnostic
contracts and legal `_Generic` executions. Two separate correctness gaps
remain; passing those tests is not full qualified-`typeof` support.

## `__builtin_types_compatible_p`

The builtin loses qualifiers originating *inside* `typeof` even though it
handles some explicitly written qualifiers. Six reproducible divergences are
tracked in `tests/regression/types_compatible_p/known_divergences.tsv`:
for example, GCC returns 1 for
`__builtin_types_compatible_p(__typeof__(const int *), const int *)`, while
LCCC returns 0. Conversely GCC returns 0 and LCCC 1 when the second operand
is `int *`. These constants can select incorrect code. The regression gate
checks both compilers and intentionally fails if a tracked answer changes,
so a fix must promote the corresponding rows to ordinary passing cases.

```sh
GCC=gcc-16 CCC="$PWD/target/fastbuild/lccc" \
  python3 tests/regression/check_types_compatible_p.py --verbose
```

## Volatile side effects

`volatile.c` isolates a separate issue: `typeof(source)` should make `target`
volatile. For this fixture on x86-64 at `-O2`, GCC 16.2 emits two reads of
`target` in both functions; LCCC emits one in `read_twice` and none in
`discard_twice`. Eliding volatile accesses is a silent correctness defect,
not an optimization win. This fixture is not a passing regression until the
volatile type and access semantics are fixed end to end.

A separate loss of qualifiers on ordinary file- and block-scope `extern`
declarations has been repaired. The existing volatile-access gate now checks
extern scalar, pointer, array, member and TLS accesses and a nonvolatile
control, as well as an extern const-pointee `_Generic` execution. This does
not establish volatile-`typeof` support.

```sh
gcc-16 -std=gnu17 -O2 -S engineering/evidence/typeof-qualifier-gaps-2026-10-28/volatile.c -o /tmp/gcc-volatile.s
target/fastbuild/lccc -std=gnu17 -O2 -S engineering/evidence/typeof-qualifier-gaps-2026-10-28/volatile.c -o /tmp/lccc-volatile.s
# Inspect both function bodies; instruction spellings differ.
```
