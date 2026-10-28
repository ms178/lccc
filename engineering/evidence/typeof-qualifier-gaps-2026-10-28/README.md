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

`volatile.c` was the original scalar witness. Before the repair, at `-O2`,
GCC 16.2 emitted two reads of `target` in each function; LCCC emitted one in
`read_twice` and none in `discard_twice`. Scalar volatility inherited directly
from a named object or a non-pointer type-name is now propagated in sema and
lowering, before any multi-declarator shadowing. This also avoids folding
explicitly const objects whose `typeof` operand makes them volatile. The
volatile-access gate checks global, local, extern, parameter and pointee
accesses, `typeof_unqual`, shadowing and nonvolatile controls at five
optimization levels against the same expected x86-64 accesses as GCC 16.2.

**Important remaining defects:** `volatile-remaining.c` shows three paths
still losing volatility: a dereferenced `typeof` operand, a volatile pointer
object used as the operand, and a typedef of `typeof(source)`. At `-O2`, GCC
16.2 emits two reads for each of `f`, `g`, and `h`; this LCCC revision emits
none. These are silent wrong-code cases, not optimization wins; scalar
coverage is not complete qualified-`typeof` support.

Cross-target inspection also found an independent backend defect in ordinary
volatile locals and parameters, without any `typeof`:
`target-local-volatile.c` emits no actual local reads on i686 or RISC-V at
`-O2`, while x86-64 and AArch64 retain them. The scalar gate is an x86-64
assembly gate and does not establish volatile-local correctness elsewhere.

A separate loss of qualifiers on ordinary file- and block-scope `extern`
declarations has also been repaired. The volatile-access gate checks extern
scalar, pointer, array, member and TLS accesses and a nonvolatile control, as
well as an extern const-pointee `_Generic` execution.

```sh
GCC=gcc-16 CCC="$PWD/target/fastbuild/lccc" bash tests/regression/check_volatile_access_semantics.sh
python3 tests/regression/check_typeof_scalar_volatile.py --compiler gcc-16
gcc-16 -O2 -S engineering/evidence/typeof-qualifier-gaps-2026-10-28/volatile-remaining.c -o /tmp/gcc-volatile.s
target/fastbuild/lccc -O2 -S engineering/evidence/typeof-qualifier-gaps-2026-10-28/volatile-remaining.c -o /tmp/lccc-volatile.s
# Inspect f, g and h in both outputs; instruction spellings differ.
```
