#!/usr/bin/env python3
"""GCC 16.2's prototypes of its library builtins, as a table lccc reads.

Why
---
`__builtin_sqrt` is `double(double)` whether or not a header declared
`sqrt`: a library builtin carries its prototype. lccc mapped each builtin to
the library function's name and took the types from whatever the translation
unit happened to say, with hand-written tables for the results of some and
for the `size_t` parameters of others. Where neither said anything the call
had no prototype at all:

    __builtin_sqrt(n)                    an `int` n went out in %edi, and sqrt
                                         read %xmm0: garbage
    __builtin_memset(p, 'x', 3.9)        the length was the bits of the double
    __builtin_strlen(s) > -1             true: the result was a signed long
    __builtin_strchr(s, 'b')[1]          read at s + 1 + 8: the result was an
                                         `int`, not a `char *`

And the builtins nobody had listed did not exist (`__builtin_tan`,
`__builtin_atan2`, `__builtin_lround`, `__builtin_isdigit`: 234 of the
library builtins GCC has among the functions asked about below).

How
---
GCC is asked for the type of each builtin, by converting it to an `int` and
reading the type out of the diagnostic:

    void f(void) { int v = __builtin_strchr; }
    initialization of 'int' from 'char * (*)(const char *, int)' makes ...

once for x86-64 and once for i686. A type that is `long unsigned int` on the
one and `unsigned int` on the other is `size_t`; `__va_list_tag *` and
`char *` is `va_list`. Anything this script has no name for stops it.

The names are the builtins lccc registers as aliases of library functions
(`BuiltinInfo::simple` in src/frontend/sema/builtins.rs) and the functions
of the C library GCC may know (CANDIDATES below). A name GCC gives no type
(a builtin that "must be directly called") has no row; an alias that is
lccc's own (OWN below) has the row written here, and GCC must not know it.
`__attribute__((noreturn))` is part of the type GCC prints and is kept: a
call of `__builtin_abort` ends the path whether or not <stdlib.h> said so.

The test
--------
The same table is written out a second time as a C program,
tests/regression/builtin_library_arguments.c: every builtin whose parameters
and result are numbers, called once with an argument of another type in each
position (an `int` where a `double` is expected, a `double` for an `int`, a
real value or a complex one of another width for a complex parameter), and
the result printed with its size. The regression corpus compiles it with GCC
and with lccc and compares the two outputs, so a builtin added to the table
is run against GCC without anyone writing its test.

    scripts/gen_libc_builtin_types.py --write          (needs gcc-16)
    scripts/gen_libc_builtin_types.py --check
    scripts/gen_libc_builtin_types.py --self-test
"""

from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BUILTINS = ROOT / "src/frontend/sema/builtins.rs"
TABLE = BUILTINS.with_name("libc_builtin_types.rs")
ARGUMENTS = ROOT / "tests/regression/builtin_library_arguments.c"
VERSION = "16.2.0"
BATCH = 40

sys.path.insert(0, str(ROOT / "scripts"))
import check_libc_builtin_arity as arity  # noqa: E402

MATH = ("acos acosh asin asinh atan atan2 atanh cbrt ceil copysign cos cosh erf erfc exp exp2 "
        "expm1 fabs fdim floor fma fmax fmin fmod frexp hypot ilogb ldexp lgamma llrint llround "
        "log log10 log1p log2 logb lrint lround modf nan nearbyint nextafter nexttoward pow "
        "remainder remquo rint round scalbln scalbn sin sinh sqrt tan tanh tgamma trunc "
        "exp10 pow10 drem gamma j0 j1 jn y0 y1 yn significand sincos scalb lgamma_r gamma_r")
COMPLEX = ("cabs cacos cacosh carg casin casinh catan catanh ccos ccosh cexp cimag clog conj "
           "cpow cproj creal csin csinh csqrt ctan ctanh")
OTHERS = (
    "memchr memcmp memcpy memmove mempcpy memset memrchr rawmemchr strcat strchr strcmp strcpy "
    "strcspn strdup strndup strlen strncat strncmp strncpy strnlen strpbrk strrchr strspn "
    "strstr stpcpy stpncpy strcasecmp strncasecmp strerror strtok strxfrm strcoll index rindex "
    "bcmp bcopy bzero ffs ffsl ffsll "
    "printf fprintf sprintf snprintf vprintf vfprintf vsprintf vsnprintf puts putchar fputs "
    "fputc fwrite putc scanf fscanf sscanf vscanf vfscanf vsscanf printf_unlocked "
    "fprintf_unlocked putchar_unlocked puts_unlocked fputs_unlocked fputc_unlocked "
    "fwrite_unlocked putc_unlocked "
    "abs labs llabs imaxabs malloc calloc realloc free aligned_alloc alloca exit _Exit _exit "
    "abort atexit "
    "isalnum isalpha isascii isblank iscntrl isdigit isgraph islower isprint ispunct isspace "
    "isupper isxdigit toascii tolower toupper iswalnum iswalpha iswblank iswcntrl iswdigit "
    "iswgraph iswlower iswprint iswpunct iswspace iswupper iswxdigit towlower towupper "
    "execl execlp execle execv execvp execve fork gettext dgettext dcgettext strfmon strftime")
# The object-size checking twins of <string.h> and <stdio.h> (_FORTIFY_SOURCE):
# `__builtin___memcpy_chk` is the library's `__memcpy_chk`.
CHECKED = ("memcpy memmove mempcpy memset stpcpy stpncpy strcat strcpy strncat strncpy "
           "snprintf sprintf vsnprintf vsprintf fprintf printf vfprintf vprintf")
# `_Float128`: the functions lccc has a lowering for, and `nanf128`, which a
# NaN builtin with a payload that is no literal calls; each also under the
# name <quadmath.h> uses (`__builtin_fabsq`).
BINARY128 = "copysignf128 fabsf128 nanf128 copysignq fabsq nanq"
CANDIDATES = sorted(
    {f"__builtin_{name}{suffix}" for name in (MATH + " " + COMPLEX).split()
     for suffix in ("", "f", "l")}
    | {f"__builtin_{name}" for name in (OTHERS + " " + BINARY128).split()}
    | {f"__builtin___{name}_chk" for name in CHECKED.split()})

# Aliases lccc registers that GCC 16.2 has no builtin for, as GCC would print
# the type of the library function on x86-64 and on i686.
OWN = {
    name: (text, text) for name, text in (
        ("__builtin_dprintf", "int (*)(int, const char *, ...)"),
        ("__builtin___dprintf_chk", "int (*)(int, int, const char *, ...)"),
    )
}

# (the type on x86-64, the type on i686) -> the variant of `LibcType`.
TYPES = {
    ("void", "void"): "Void",
    ("int", "int"): "Int",
    ("unsigned int", "unsigned int"): "UnsignedInt",
    ("long int", "long int"): "Long",
    ("long long int", "long long int"): "LongLong",
    ("long unsigned int", "unsigned int"): "Size",
    ("long int", "int"): "SignedSize",
    ("long int", "long long int"): "IntMax",
    ("float", "float"): "Float",
    ("double", "double"): "Double",
    ("long double", "long double"): "LongDouble",
    ("_Float128", "_Float128"): "Float128",
    ("_Complex float", "_Complex float"): "ComplexFloat",
    ("_Complex double", "_Complex double"): "ComplexDouble",
    ("_Complex long double", "_Complex long double"): "ComplexLongDouble",
    ("void *", "void *"): "VoidPtr",
    ("const void *", "const void *"): "ConstVoidPtr",
    ("char *", "char *"): "CharPtr",
    ("const char *", "const char *"): "ConstCharPtr",
    ("int *", "int *"): "IntPtr",
    ("float *", "float *"): "FloatPtr",
    ("double *", "double *"): "DoublePtr",
    ("long double *", "long double *"): "LongDoublePtr",
    ("char * const*", "char * const*"): "CharPtrConstPtr",
    ("__va_list_tag *", "char *"): "VaList",
}
ATTRIBUTE = re.compile(r"__attribute__\(\([a-z_ ,]*\)\) ?")
DIAGNOSTIC = re.compile(r"(?:^|/)p\.c:(\d+):\d+: (?:error|warning): initialization of 'int' "
                        r"from '(.*)' makes integer from pointer without a cast")


def is_noreturn(text: str) -> bool:
    return any("noreturn" in attribute for attribute in ATTRIBUTE.findall(text))


def parse_type(text: str) -> tuple[str, list[str], bool]:
    """`char * (*)(const char *, int)` -> ("char *", ["const char *", "int"], False)."""
    text = ATTRIBUTE.sub("", text).strip()
    match = re.fullmatch(r"(.*?) ?\(\*\)\((.*)\)", text)
    if not match:
        raise ValueError(f"not a pointer to a function: {text!r}")
    result = " ".join(match.group(1).split())
    parameters = [" ".join(p.split()) for p in match.group(2).split(",")]
    variadic = parameters[-1] == "..."
    if variadic:
        parameters.pop()
    if parameters == ["void"]:
        parameters = []
    if "..." in parameters or any("(" in p for p in parameters):
        raise ValueError(f"a parameter this script does not read: {text!r}")
    return result, parameters, variadic


def variant(wide: str, narrow: str) -> str:
    try:
        return TYPES[(wide, narrow)]
    except KeyError:
        raise ValueError(f"no name for the type {wide!r} (x86-64) / {narrow!r} (i686)") from None


def combine(wide: str, narrow: str) -> tuple[str, list[str], bool, bool]:
    """One prototype from GCC's two answers, in `LibcType` variants:
    (result, parameters, variadic, noreturn)."""
    w_result, w_params, w_variadic = parse_type(wide)
    n_result, n_params, n_variadic = parse_type(narrow)
    if (len(w_params) != len(n_params) or w_variadic != n_variadic
            or is_noreturn(wide) != is_noreturn(narrow)):
        raise ValueError(f"the two targets disagree on the shape: {wide!r} / {narrow!r}")
    return (variant(w_result, n_result),
            [variant(w, n) for w, n in zip(w_params, n_params)], w_variadic, is_noreturn(wide))


def probe(gcc: str, names: list[str], mode: str, directory: Path) -> dict[str, str]:
    """The type GCC gives each name that has one."""
    found: dict[str, str] = {}
    source = directory / "p.c"
    for start in range(0, len(names), BATCH):
        batch = names[start:start + BATCH]
        source.write_text("void probe(void) {\n" + "".join(
            f"    int v{index} = {name}; (void)v{index};\n"
            for index, name in enumerate(batch)) + "}\n")
        result = subprocess.run([gcc, mode, "-std=gnu17", "-fsyntax-only", str(source)],
                                capture_output=True, text=True, cwd=directory)
        for line in result.stderr.splitlines():
            match = DIAGNOSTIC.search(line)
            if match and 0 <= int(match.group(1)) - 2 < len(batch):
                found.setdefault(batch[int(match.group(1)) - 2], match.group(2))
    return found


def measure(gcc: str, names: list[str]) -> dict[str, tuple[str, list[str], bool, bool]]:
    with tempfile.TemporaryDirectory() as tmp:
        wide = probe(gcc, names, "-m64", Path(tmp))
        narrow = probe(gcc, names, "-m32", Path(tmp))
    if set(wide) != set(narrow):
        raise ValueError(f"a builtin of one target only: {sorted(set(wide) ^ set(narrow))}")
    if len(wide) < 300:
        raise ValueError(f"{gcc} gave a type for {len(wide)} names: is it GCC {VERSION}?")
    known = sorted(set(OWN) & set(wide))
    if known:
        raise ValueError(f"GCC has a builtin for {known}: take the row out of OWN")
    table = {name: combine(wide[name], narrow[name]) for name in wide}
    table.update({name: combine(*types) for name, types in OWN.items()})
    return table


def names_asked() -> list[str]:
    registered = [name for name in arity.registered(BUILTINS.read_text())
                  if name.startswith("__builtin_")]
    return sorted(set(registered) | set(CANDIDATES) | set(OWN))


def render(table: dict[str, tuple[str, list[str], bool, bool]]) -> str:
    variants = list(dict.fromkeys(TYPES.values()))
    lines = [
        f"//! GCC {VERSION}'s prototypes of its library builtins.",
        "//!",
        "//! Generated by `scripts/gen_libc_builtin_types.py --write`; never edited by hand.",
        f"//! {len(table)} builtins. A builtin carries its prototype whether or not a header",
        "//! declared the library function: these are the types its arguments are",
        "//! converted to and the type its call has.",
        "",
        "use crate::common::types::{AddressSpace, CType, PtrQuals, target_is_32bit};",
        "",
        "/// A type of a library builtin's prototype, as GCC names it on x86-64 and on",
        "/// i686 (`Size` is `long unsigned int` on the one and `unsigned int` on the",
        "/// other, `IntMax` `long int` and `long long int`; `VaList` is",
        "/// `__va_list_tag *` and `char *`).",
        "#[derive(Clone, Copy, Debug, PartialEq, Eq)]",
        "pub(crate) enum LibcType {",
        *[f"    {name}," for name in variants],
        "}",
        "",
        "/// The prototype of one library builtin.",
        "#[derive(Debug)]",
        "pub(crate) struct LibcPrototype {",
        "    pub(crate) result: LibcType,",
        "    pub(crate) parameters: &'static [LibcType],",
        "    pub(crate) variadic: bool,",
        "    /// A call of it does not return (`__builtin_abort`).",
        "    pub(crate) noreturn: bool,",
        "}",
        "",
        "impl LibcType {",
        "    /// The C type, with the `const` GCC's prototype has on what a pointer",
        "    /// points to: an argument is judged against it (`__builtin_strcpy(d, s)`",
        "    /// with a `const char *d` discards the qualifier).",
        "    pub(crate) fn ctype(self) -> CType {",
        "        let pointer = |to: CType, cvr: u8| {",
        "            CType::Pointer(Box::new(to), PtrQuals::recorded(AddressSpace::Default, cvr))",
        "        };",
        "        match self {",
        "            LibcType::Void => CType::Void,",
        "            LibcType::Int => CType::Int,",
        "            LibcType::UnsignedInt => CType::UInt,",
        "            LibcType::Long => CType::Long,",
        "            LibcType::LongLong => CType::LongLong,",
        "            LibcType::Size if target_is_32bit() => CType::UInt,",
        "            LibcType::Size => CType::ULong,",
        "            LibcType::SignedSize if target_is_32bit() => CType::Int,",
        "            LibcType::SignedSize => CType::Long,",
        "            LibcType::IntMax if target_is_32bit() => CType::LongLong,",
        "            LibcType::IntMax => CType::Long,",
        "            LibcType::Float => CType::Float,",
        "            LibcType::Double => CType::Double,",
        "            LibcType::LongDouble => CType::LongDouble,",
        "            LibcType::Float128 => CType::Float128,",
        "            LibcType::ComplexFloat => CType::ComplexFloat,",
        "            LibcType::ComplexDouble => CType::ComplexDouble,",
        "            LibcType::ComplexLongDouble => CType::ComplexLongDouble,",
        "            LibcType::VoidPtr => pointer(CType::Void, 0),",
        "            LibcType::ConstVoidPtr => pointer(CType::Void, PtrQuals::CONST),",
        "            LibcType::CharPtr => pointer(CType::plain_char(), 0),",
        "            LibcType::ConstCharPtr => pointer(CType::plain_char(), PtrQuals::CONST),",
        "            LibcType::IntPtr => pointer(CType::Int, 0),",
        "            LibcType::FloatPtr => pointer(CType::Float, 0),",
        "            LibcType::DoublePtr => pointer(CType::Double, 0),",
        "            LibcType::LongDoublePtr => pointer(CType::LongDouble, 0),",
        "            LibcType::CharPtrConstPtr => "
        "pointer(pointer(CType::plain_char(), 0), PtrQuals::CONST),",
        "            // i686's `va_list`. A parameter has the target's:",
        "            // `parameter_ctype`.",
        "            LibcType::VaList => pointer(CType::plain_char(), 0),",
        "        }",
        "    }",
        "",
        "    /// The type of a parameter of this type, where `va_list` is the",
        "    /// target's `__builtin_va_list`: an array type is adjusted to a pointer",
        "    /// to its element (x86-64's one `__va_list_tag`), as for any parameter.",
        "    pub(crate) fn parameter_ctype(self, va_list: Option<&CType>) -> CType {",
        "        match (self, va_list) {",
        "            (LibcType::VaList, Some(CType::Array(element, _))) => {",
        "                CType::Pointer(element.clone(), PtrQuals::GENERIC)",
        "            }",
        "            (LibcType::VaList, Some(va_list)) => va_list.clone(),",
        "            _ => self.ctype(),",
        "        }",
        "    }",
        "}",
        "",
        "/// The prototype of the library builtin spelled `name` (`__builtin_strchr`).",
        "pub(crate) fn libc_builtin_prototype(name: &str) -> Option<&'static LibcPrototype> {",
        "    let index = PROTOTYPES.binary_search_by_key(&name, |row| row.0).ok()?;",
        "    Some(&PROTOTYPES[index].1)",
        "}",
        "",
        "/// Every builtin of the table, in the order of its names.",
        "pub(crate) fn libc_builtin_names() -> impl Iterator<Item = &'static str> {",
        "    PROTOTYPES.iter().map(|row| row.0)",
        "}",
        "",
        "#[rustfmt::skip]",
        "const PROTOTYPES: &[(&str, LibcPrototype)] = &[",
    ]
    for name, (result, parameters, variadic, noreturn) in sorted(table.items()):
        listed = ", ".join(f"LibcType::{p}" for p in parameters)
        lines.append(f'    ("{name}", LibcPrototype {{ result: LibcType::{result}, '
                     f"parameters: &[{listed}], variadic: {str(variadic).lower()}, "
                     f"noreturn: {str(noreturn).lower()} }}),")
    lines += ["];", ""]
    return "\n".join(lines)


REAL_FLOATING = ("Float", "Double", "LongDouble")
INTEGERS = ("Int", "UnsignedInt", "Long", "LongLong", "Size", "SignedSize", "IntMax")
COMPLEXES = ("ComplexFloat", "ComplexDouble", "ComplexLongDouble")
# The test's variables: the name, its `LibcType` (None: a type no parameter
# has), the declaration.
VARIABLES = (
    ("an_int", "Int", "int an_int = 2;"),
    ("an_unsigned", "UnsignedInt", "unsigned an_unsigned = 3;"),
    ("a_long", "Long", "long a_long = 1;"),
    ("a_char", None, "signed char a_char = 2;"),
    ("a_float", "Float", "float a_float = 1.75f;"),
    ("a_double", "Double", "double a_double = 2.9;"),
    ("a_long_double", "LongDouble", "long double a_long_double = 3.6L;"),
    ("a_letter", "Double", "double a_letter = 65.9;"),
    ("a_complex_float", "ComplexFloat", "_Complex float a_complex_float = 0.5f + 1.25fi;"),
    ("a_complex", "ComplexDouble", "_Complex double a_complex = 1.5 + 0.5i;"),
    ("a_complex_long_double", "ComplexLongDouble",
     "_Complex long double a_complex_long_double = 2.0L + 0.25Li;"),
)
# Arguments of another type than the parameter's, by the parameter's type;
# the test takes them in turn.
OTHER_ARGUMENTS = {
    "Float": ("an_int", "an_unsigned", "a_long", "a_double", "a_char"),
    "Double": ("an_int", "an_unsigned", "a_long", "a_float", "a_char"),
    "LongDouble": ("an_int", "a_double", "a_long", "a_float", "a_char"),
    **{name: ("a_double", "a_float", "a_long_double") for name in INTEGERS},
    "ComplexFloat": ("an_int", "a_double", "a_complex"),
    "ComplexDouble": ("an_int", "a_float", "a_complex_float", "a_complex_long_double"),
    "ComplexLongDouble": ("a_double", "a_complex_float", "a_complex"),
}
# Not called by the test: these write to the output themselves, and glibc
# 2.27 took `pow10` out of the library.
NOT_CALLED = {"__builtin_putchar", "__builtin_putchar_unlocked",
              "__builtin_pow10", "__builtin_pow10f", "__builtin_pow10l"}
CHARACTER_CLASS = re.compile(r"__builtin_(is|to)[a-z]+$")


def argument_calls(table: dict[str, tuple[str, list[str], bool, bool]]) -> list[tuple[str, str]]:
    """(the call, the type of its result) for every builtin the test can call."""
    calls = []
    numbers = REAL_FLOATING + INTEGERS + COMPLEXES
    for name, (result, parameters, variadic, noreturn) in sorted(table.items()):
        if (variadic or noreturn or name in NOT_CALLED or name.endswith("_chk")
                or not parameters or result not in numbers
                or any(parameter not in numbers for parameter in parameters)):
            continue
        if CHARACTER_CLASS.match(name) and parameters == ["Int"]:
            arguments = ["a_letter"]  # 65.9 is 'A': a class and a case to answer about
        else:
            arguments = [OTHER_ARGUMENTS[parameter][(len(calls) + position)
                                                    % len(OTHER_ARGUMENTS[parameter])]
                         for position, parameter in enumerate(parameters)]
        calls.append((f"{name}({', '.join(arguments)})", result))
    return calls


def argument_test(table: dict[str, tuple[str, list[str], bool, bool]]) -> str:
    """The C program: one call a line, the result through a `volatile` object
    of the call's type (so an x87 result is rounded to that type before it is
    printed, as the type says it is), then its size."""
    calls = argument_calls(table)
    lines = [
        "/* Every library builtin whose parameters and result are numbers, called with",
        " * an argument of another type in each position.",
        " *",
        f" * Generated by `scripts/gen_libc_builtin_types.py --write` from GCC {VERSION}'s",
        f" * prototypes; never edited by hand. {len(calls)} calls.",
        " *",
        " * A library builtin carries its prototype: `__builtin_sqrt(n)` converts an",
        " * `int` to `double`, `__builtin_ldexp(x, 2.9)` a `double` to `int`,",
        " * `__builtin_cexp(n)` a real value to a complex one, with no header included",
        " * (this file includes none) and whatever a header declares (the `_declared`",
        " * twin includes them all). Each call prints its result and the size of its",
        " * type. GCC's output is the reference.",
        " *",
        " * Before the prototypes were GCC's, 234 of these builtins were undeclared",
        " * functions and the rest passed the argument as it was: an `int` in a",
        " * general register to a function that reads %xmm0. */",
        "",
        *[declaration for _, _, declaration in VARIABLES],
        "",
        "#define NAME(call) __builtin_printf(\"%-58s \", #call)",
        "#define REAL(call) do { \\",
        "        volatile __typeof__(call) result_ = (call); \\",
        "        NAME(call); \\",
        "        __builtin_printf(\"%.9Lg %d\\n\", (long double)result_, (int)sizeof(call)); \\",
        "    } while (0)",
        "#define INTEGER(call) do { \\",
        "        volatile __typeof__(call) result_ = (call); \\",
        "        NAME(call); \\",
        "        __builtin_printf(\"%lld %d\\n\", (long long)result_, (int)sizeof(call)); \\",
        "    } while (0)",
        "#define COMPLEX(call) do { \\",
        "        volatile __typeof__(call) result_ = (call); \\",
        "        NAME(call); \\",
        "        __builtin_printf(\"%.7Lg %.7Lg %d\\n\", (long double)__real__ result_, \\",
        "                         (long double)__imag__ result_, (int)sizeof(call)); \\",
        "    } while (0)",
        "",
        "int main(void) {",
    ]
    shape = {**dict.fromkeys(REAL_FLOATING, "REAL"), **dict.fromkeys(INTEGERS, "INTEGER"),
             **dict.fromkeys(COMPLEXES, "COMPLEX")}
    lines += [f"    {shape[result]}({call});" for call, result in calls]
    lines += ["    return 0;", "}", ""]
    return "\n".join(lines)


def self_test() -> int:
    assert parse_type("char * (*)(const char *, int)") == ("char *", ["const char *", "int"], False)
    assert parse_type("__attribute__((const)) double (*)(double)") == ("double", ["double"], False)
    assert parse_type("__attribute__((noreturn)) void (*)(void)") == ("void", [], False)
    assert parse_type("int (*)(const char *, ...)") == ("int", ["const char *"], True)
    assert parse_type("int (*)(char *, long unsigned int,  const char *, ...)") == (
        "int", ["char *", "long unsigned int", "const char *"], True)
    assert parse_type("void * (*)(long unsigned int,  long unsigned int)") == (
        "void *", ["long unsigned int", "long unsigned int"], False)
    assert parse_type("int (*)(const char *, char * const*)") == (
        "int", ["const char *", "char * const*"], False)
    for bad in ("double", "int (*)(int (*)(void))"):
        try:
            parse_type(bad)
        except ValueError:
            continue
        raise AssertionError(f"{bad!r} was read")
    assert combine("long unsigned int (*)(const char *)", "unsigned int (*)(const char *)") == (
        "Size", ["ConstCharPtr"], False, False)
    assert combine("long int (*)(double)", "long int (*)(double)") == (
        "Long", ["Double"], False, False)
    assert combine("long int (*)(long int)", "long long int (*)(long long int)") == (
        "IntMax", ["IntMax"], False, False)
    assert combine("int (*)(const char *, __va_list_tag *)", "int (*)(const char *, char *)") == (
        "Int", ["ConstCharPtr", "VaList"], False, False)
    noreturn = "__attribute__((noreturn)) void (*)(int)"
    assert combine(noreturn, noreturn) == ("Void", ["Int"], False, True)
    assert not is_noreturn("__attribute__((const)) double (*)(double)")
    for name, types in OWN.items():
        assert name.startswith("__builtin_") and combine(*types)
    for wide, narrow in (("int (*)(int)", "int (*)(int, int)"),
                         ("int (*)(int, ...)", "int (*)(int)"),
                         ("short int (*)(int)", "short int (*)(int)"),
                         (noreturn, "void (*)(int)"),
                         ("long unsigned int (*)(int)", "long unsigned int (*)(int)")):
        try:
            combine(wide, narrow)
        except ValueError:
            continue
        raise AssertionError(f"{wide!r} / {narrow!r} was combined")
    text = render({"__builtin_b": ("Int", ["Double", "Size"], False, True),
                   "__builtin_a": ("CharPtr", [], True, False)})
    assert text.index('"__builtin_a"') < text.index('"__builtin_b"')
    assert ("parameters: &[LibcType::Double, LibcType::Size], variadic: false, noreturn: true"
            in text)
    assert "parameters: &[], variadic: true, noreturn: false" in text
    assert text.count('\n    ("__builtin_') == 2
    # The test: only what takes numbers and gives one, no argument of the
    # parameter's own type, a name for every variable.
    types = {name: kind for name, kind, _ in VARIABLES}
    for parameter, others in OTHER_ARGUMENTS.items():
        assert others and all(types[name] != parameter for name in others), parameter
    assert set(OTHER_ARGUMENTS) == set(REAL_FLOATING + INTEGERS + COMPLEXES)
    sample = {
        "__builtin_sqrtf": ("Float", ["Float"], False, False),
        "__builtin_ldexp": ("Double", ["Double", "Int"], False, False),
        "__builtin_cexp": ("ComplexDouble", ["ComplexDouble"], False, False),
        "__builtin_isdigit": ("Int", ["Int"], False, False),
        "__builtin_strlen": ("Size", ["ConstCharPtr"], False, False),
        "__builtin_frexp": ("Double", ["Double", "IntPtr"], False, False),
        "__builtin_printf": ("Int", ["ConstCharPtr"], True, False),
        "__builtin_exit": ("Void", ["Int"], False, True),
        "__builtin_putchar": ("Int", ["Int"], False, False),
        "__builtin_fork": ("Int", [], False, False),
        "__builtin___memset_chk": ("Int", ["Int"], False, False),
    }
    calls = argument_calls(sample)
    assert calls == [("__builtin_cexp(an_int)", "ComplexDouble"),
                     ("__builtin_isdigit(a_letter)", "Int"),
                     ("__builtin_ldexp(a_long, a_double)", "Double"),
                     ("__builtin_sqrtf(a_double)", "Float")], calls
    program = argument_test(sample)
    assert "    COMPLEX(__builtin_cexp(an_int));\n" in program
    assert "    INTEGER(__builtin_isdigit(a_letter));\n" in program
    assert "    REAL(__builtin_sqrtf(a_double));\n" in program
    assert "#include" not in program and "4 calls." in program
    assert "__builtin_tan" in CANDIDATES and "__builtin_tanf" in CANDIDATES
    assert "__builtin_strnlen" in CANDIDATES and "__builtin_strnlenf" not in CANDIDATES
    assert "__builtin___memcpy_chk" in CANDIDATES
    assert len(set(TYPES.values())) == len(TYPES)
    print("gen_libc_builtin_types: self-test OK")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--gcc", default=os.environ.get("GCC", "gcc-16"))
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--write", action="store_true")
    mode.add_argument("--check", action="store_true")
    mode.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        return self_test()
    version = subprocess.run([args.gcc, "-dumpfullversion"], capture_output=True, text=True)
    if version.returncode != 0 or version.stdout.strip() != VERSION:
        print(f"gen_libc_builtin_types: {args.gcc} is {version.stdout.strip() or 'missing'}; "
              f"the table is GCC {VERSION}'s", file=sys.stderr)
        return 2
    table = measure(args.gcc, names_asked())
    outputs = ((TABLE, render(table)), (ARGUMENTS, argument_test(table)))
    if args.write:
        for path, text in outputs:
            path.write_text(text)
        print(f"wrote {TABLE.relative_to(ROOT)} ({len(table)} builtins) and "
              f"{ARGUMENTS.relative_to(ROOT)} ({len(argument_calls(table))} calls)")
        return 0
    stale = [path for path, text in outputs if not path.is_file() or path.read_text() != text]
    if stale:
        print(f"gen_libc_builtin_types: {', '.join(str(p.relative_to(ROOT)) for p in stale)} "
              f"is not what GCC {VERSION} says; run with --write and review the diff",
              file=sys.stderr)
        return 1
    print(f"gen_libc_builtin_types: {TABLE.relative_to(ROOT)} and "
          f"{ARGUMENTS.relative_to(ROOT)} match GCC {VERSION} ({len(table)} builtins, "
          f"{len(argument_calls(table))} calls)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
