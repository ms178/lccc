# Operand constraints against GCC 16.2: what was measured, closed and left

Date: 2026-11-28. Found through the SIMD headers (PR #1211), not particular
to them.

## How it was found

The value differential of the x86 intrinsics began to pass scalar arguments
in objects of another type than the parameter's
(`docs/x86-isa-permissions-and-simd-abi.md` §2.8). A `double` handed to a
mask parameter reached a header macro `(__k) & compare(...)`, and the
compiler stopped:

    ccc: internal error: unsupported float binop: And on type F64

The same happens for `k & __builtin_popcount(a)` with a `double k`, with no
header involved. `%`, the shifts and the bitwise operators take integers
only; the analyzer checked that where it knew both operand types, and it
has no type for the result of a builtin. The code generator has no
floating `and`.

## The tool

`scripts/operand_constraint_matrix.py` asks the question for every operator
and every kind of operand: one translation unit per cell, both compilers
under `-fsyntax-only`, the cell is the first error of each.

| table | cells | differ before | differ now |
|---|---|---|---|
| unary: 20 operand kinds, 10 operators | 200 | 55 | 10 |
| binary: 28 kinds squared, 18 operators | 14,112 | 3,134 | 1,139 |

The ten unary cells left are `_BitInt`, a type lccc does not have. Of the
binary cells, 70 are both compilers refusing in other words; in the other
1,069 lccc accepts a program GCC refuses. In no cell, before or after,
does lccc refuse a program GCC accepts. (The binary count was 2,330 when
this work landed, 62 worded otherwise and 2,268 accepted; the library
builtins' prototypes closed 1,199 of those, and eight of them are refused
in other words now.)

## What is closed

* Unary operators (C11 6.5.3.3): a pointer has a truth value and nothing
  else, so `+p`, `-p`, `~p`, `__real__ p` and `__imag__ p` are "wrong type
  argument" (an array and a function designator are pointers by then);
  `~` takes an integer, a complex value (GNU: the conjugate) or a vector of
  integers; a GNU vector has `+`, `-`, `++`, `--` and neither a truth
  value nor parts. lccc compiled all of these: `~5.7` was -0.7875, the
  complement of the bits.
* `++`, `--` and assignment of an array or a function designator: "lvalue
  required as increment operand", "lvalue required as left operand of
  assignment". lccc stepped, or stored through, the address.
* The operators of integers only with one operand floating for certain:
  an exactly typed operand, or a builtin declared to return a floating
  type. The analyzer judges them; the other operand is only named.
* The same where the analyzer has no type for the builtin at all: the
  lowering knows every operand's type and gives GCC's diagnostic at the
  operator, for the binary and the compound-assignment forms. The code
  generator no longer sees an integer-only operation on a floating type.
* An ordering of complex values (`z < 1`).

## What is left, counted

The gate holds each number as a ceiling (`BINARY_OPEN` in the script).

| operand | cells | what it takes |
|---|---|---|
| the result of a builtin | 295 (1,494 at first) | A type for a builtin call that the constraints may trust. The analyzer has none because a builtin's declared type is not always what it yields (the vector builtins yield an address) and some return the type of an argument. A library builtin now has the result of GCC's prototype (`FOLLOWUP-2026-11-28-library-builtin-prototypes.md`), which closed 1,199 cells; the rest are builtins that are no library function (`__builtin_popcount`), and the same table closes them once it has their rows. |
| a GNU vector | 752 | GCC's scalar-to-vector rule: a scalar operand is converted to the vector's element type when that loses nothing, and refused otherwise ("conversion of scalar 'int' to vector '__vector(4) float' involves truncation"); `&&` and `||` take no vector. |
| pointers | 22 | The difference of pointers to incompatible types (`int *` minus `void *`). |

Also seen and not in this work:

* `_BitInt` is not implemented.
* Assignments and `&` to things that are no lvalues are refused in Clang's
  words ("expression is not assignable") where every other operand
  diagnostic is in GCC's.
* A diagnostic about an operand does not mark the expression as erroneous,
  so a second one about the same expression can follow (`return ~p;` in a
  function returning `int` also reports the conversion of the pointer).

## Gates

* `scripts/operand_constraint_matrix.py --check` (job `backend`): no cell
  is a valid program refused or an internal error; the unary table agrees
  but for the recorded `_BitInt` cells; the binary acceptances are at most
  the recorded counts.
* `tests/regression/check_operand_diagnostics.sh`: line, column and
  message of 54 new rows recorded from GCC 16.2, seven rows for the
  lowering, 24 legal forms.
* A whole-repository differential before the change was accepted: 3,673 C
  files under `-fsyntax-only`, 2,895 accepted before and after, none newly
  rejected.
