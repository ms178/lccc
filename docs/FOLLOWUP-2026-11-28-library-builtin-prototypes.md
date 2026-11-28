# A library builtin is called by its own prototype

Date: 2026-11-28. Found while testing argument conversion of the x86
intrinsics (PR #1211); the defect is in every target and is older than the
PR.

## What was wrong

`__builtin_sqrt` is `double(double)` in GCC whether or not `<math.h>` was
included: the builtin carries a prototype, the argument is converted to the
parameter as if by assignment, and the call has the prototype's result type.

lccc mapped each such builtin to the library function's name and had the
types in three places written by hand: the results of some builtins in the
analyzer, the results of some in the lowering, the `size_t` parameters of
some for the call. Conversions were done between integers only. What none of
the lists named had an `int` result and no parameter types. With no header
included, against GCC 16.2:

| call | GCC | lccc |
|---|---|---|
| `__builtin_sqrt(n)`, `int n = 16` | 4 | NaN (the `int` went out in `%edi`; `sqrt` read `%xmm0`) |
| `__builtin_pow(2, 10)` | 1024 | 1 |
| `__builtin_ldexp(3, 2.9)` | 12 | 23.2 (the exponent was the bits of 2.9) |
| `__builtin_memset(p, 'x', 3.9)` | three bytes set | none, or a fault |
| `__builtin_strchr(s, 98.9)` | found | not found |
| `__builtin_strchr(s, 'b')[1]` | `c` | a read 8 bytes further: the result was an `int` |
| `__builtin_strlen(s) - 6 > 0` | 1 | 0: the result was signed |
| `sizeof(*__builtin_strchr(s, 'a'))` | 1 | 8 |
| `__builtin_tan(x)`, `__builtin_atan2`, `__builtin_lround`, `__builtin_strnlen`, `__builtin_isdigit`, ... | calls | "implicit declaration of function": 234 builtins did not exist |
| `__builtin_strlen(5)`, `__builtin_sqrt("a")`, `__builtin_memcpy(d, s, p)` | errors | accepted |
| `__builtin_nanf128()` | "too few arguments" | accepted |

Two more defects surfaced on the way and are not about builtins:

* **An integer argument for a complex parameter.** `f(n)` for `double
  f(double _Complex)` and an `int n` converted `n` to a complex value and
  then converted the *address* of that value from `int` to the parameter:
  `cltq` on a stack address. Every integer narrower than a pointer did that
  (`char`, `_Bool`, `unsigned`; `long long` on i686). `cacos(2)` with
  `<complex.h>` was a segmentation fault.
* **`conj`, `conjf`, `conjl`** were lowered as if type-generic: the result
  had the argument's type, and a real argument came back as it was while
  the expression was typed complex. `conj(n)` was a segmentation fault;
  `__builtin_conjf(x)` of a `double x = 2.9` was 8.58993e+08.

## The table

`scripts/gen_libc_builtin_types.py` asks GCC 16.2 for the type of each
builtin by converting it to an `int` and reading the diagnostic,

    void f(void) { int v = __builtin_strchr; }
    initialization of 'int' from 'char * (*)(const char *, int)' makes ...

once for x86-64 and once for i686; the pair of answers is one type (`long
unsigned int` and `unsigned int` is `size_t`; `__va_list_tag *` and `char *`
is `va_list`). A type the script has no name for stops it.
`__attribute__((noreturn))` is kept. The output is
`src/frontend/sema/libc_builtin_types.rs`: 425 prototypes (the math, complex,
string, stdio, stdlib, ctype and wctype functions, the object-size checking
twins of `_FORTIFY_SOURCE`, and the `_Float128` ones lccc lowers). Two rows
are lccc's own, written in the script, and GCC must not know them:
`__builtin_dprintf` and `__builtin___dprintf_chk`.

Everything reads this one table:

* **Registration.** Every builtin of the table that has no lowering of its
  own is a call of the library function named by the builtin without its
  prefix (`__builtin___memcpy_chk` is `__memcpy_chk`). A unit test holds
  that no alias exists without a prototype.
* **The call.** A call of a library builtin is lowered as the call of a
  function declared with the prototype, under the builtin's own name and
  with the library function as its symbol, as if the unit had said `double
  __builtin_sqrt(double) __asm__("sqrt");`. That is the ordinary call path,
  so each argument is converted to its parameter, the tail of a variadic
  builtin is promoted, a complex or `_Float128` value travels by the
  target's convention, and a `noreturn` builtin ends the path. The arm that
  emitted these calls with conversions of its own is gone, and so are the
  special lowering of the checking twins and the one of `__builtin_cpow`.
  The symbol follows a declaration's `__asm__` label, as GCC's does
  (glibc's `sscanf` is `__isoc23_sscanf`).
* **The type of the call.** The analyzer's and the lowering's lists of
  result types lost every name the table has (69 and 150 names).
* **The arguments.** The analyzer judges each argument against its
  parameter with the checker ordinary calls use, `const` on a pointee
  included. The number of arguments is the prototype's. GCC's verdict on a
  wrong number is also measured on its own
  (`scripts/check_libc_builtin_arity.py`), and a unit test holds the two
  measurements to one answer.
* **Operand constraints.** A library builtin's call has an exact type for
  the operators' constraints: 1,199 cells of the operand matrix closed
  (`BINARY_OPEN["builtin"]` 1,494 to 295).

Builtins the table types and lccc lowers itself (`__builtin_alloca`,
`__builtin_creal`, `__builtin_conj`, `__builtin_ffs`, `__builtin_nan` with a
literal payload) keep their lowering and take the table's types.

## Tests

* `tests/regression/builtin_library_arguments.c` is written by the same
  script from the same table: every builtin whose parameters and result are
  numbers (297 of them), called with an argument of another type in each
  position, the result printed with its size. The regression corpus runs it
  against GCC at `-O2`, at `-O0`, on i686, and with every library header
  included (`_O0`, `_i686`, `_declared`). A builtin added to the table is
  tested without anyone writing the test.
* `tests/regression/builtin_library_prototypes.c`, by hand, same four
  variants (the `_declared` one with `_FORTIFY_SOURCE=2`): the builtins
  that take pointers, the variadic ones, the checking twins, a `va_list`
  passed on, `noreturn`, the type of 48 calls by name, and the complex
  parameter of a declared function.
* `scripts/gen_libc_builtin_types.py --check` (job `frontend-b`, with its
  self-test): both generated files are what GCC 16.2 says.
* Unit tests: every alias has a prototype, every prototype is registered,
  the library function is the name without the prefix, the rows the defects
  were, the arity table agrees, pointer parameters carry their qualifier.
* Measured once, by hand: 57 ill-typed calls under `-fsyntax-only`; 51 have
  GCC's first diagnostic word for word, 6 are refused in other words (see
  below). Every C file of the repository under `-fsyntax-only` before and
  after: one newly rejected (`tests/corpus/clang-c/Sema/
  overloaded-math-builtins.c`, which GCC rejects at the same four places),
  ten newly accepted.

`tests/regression/glibc_f128_builtins.c` called `__builtin_nanf128()` and
was marked lccc-only because GCC refuses that. It passes `""` now and is
compared against GCC like the rest.

## Left

* **Builtins that are no library function** (`__builtin_popcount`, `clz`,
  `ctz`, `parity`, `clrsb`, `bswap`): GCC gives them a type the same way,
  and the same table can hold them. `__builtin_bswap16(x)` still has the
  size of an `int` (GCC: 2), and the 295 operand-matrix cells are these.
* **The plain names.** `types_seed.rs` seeds signatures for library
  functions called without a declaration (`sqrt(x)` in a dialect that
  allows it) from a list written by hand, for x86-64 (`cabsl` returns a
  `double` there). The table has GCC's prototype for each under the
  builtin's name.
* **Wording.** A pointer converted to a floating type or back is "incompatible
  types (have 'int *' but expected 'double')" in every context; GCC says
  "incompatible type for argument 1 of 'f'", "incompatible types when
  initializing type ...", and so on. Declared functions have the same
  difference.
* **`const` and `pure`.** GCC's type for `__builtin_fabs` is
  `__attribute__((const))`; the script drops it. A call of a `const`
  builtin could be removed when unused and shared when repeated.
* **A constant expression.** `(float)-1 > 0` folds to 1 (the cast keeps 32
  bits of the integer and then reads them as a value). Not a builtin's
  defect; found by this work's type probe and fixed separately.
