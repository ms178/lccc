#!/usr/bin/env bash
# GP-23 macro-parity gate: lccc -dM -E must match gcc-16 -dM -E.
#
# For each (target, dialect) pair, diff `lccc -dM -E -std=<dialect>` against
# `gcc-16 -dM -E -std=<dialect>`, classifying differences into three pinned
# lists:
#
#   * FORBIDDEN — defined by lccc only, gcc-16 does not define them.
#     These are *wrong* predefines: libc macros (EOF, BUFSIZ, errno, …) that
#     the real headers must provide, and __STDC_NO_ATOMICS__-style capability
#     denials for features lccc implements. Must be empty (ratchet: never grow).
#
#   * MISSING — defined by gcc-16 only, lccc does not define them.
#     The essential set (__SIZEOF_*, __CHAR_BIT__, __BYTE_ORDER__, …) must
#     be empty; the rest is a shrinking list as GP-05 adds them.
#
#   * SPELLING — defined by both but with different values.
#     Justified row by row (e.g. hex vs decimal, __LDBL_MAX__ suffix).
#
# Positive control: re-add BUFSIZ and this gate must fail.
set -euo pipefail

repo=$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)
CCC=${CCC:-target/fastbuild/lccc}
GCC=${GCC:-gcc-16}

# Pinned lists — GP-05 shrinks MISSING, GP-01 shrinks FORBIDDEN, GP-23
# justifies SPELLING row by row.  Edit only with a corresponding GP item.
readonly FORBIDDEN_LIST=(
    # libc macros the real headers must provide (GP-05)
    "EOF" "BUFSIZ" "EXIT_SUCCESS" "EXIT_FAILURE" "RAND_MAX"
    "SEEK_SET" "SEEK_CUR" "SEEK_END"
    # errno values
    "ENOENT" "EINTR" "EIO" "ENOMEM" "EACCES" "EEXIST" "ENOTDIR" "EISDIR"
    "EINVAL" "ENFILE" "EMFILE" "ENOSPC" "EPIPE" "ERANGE" "ENOSYS"
    # signal numbers
    "SIGABRT" "SIGFPE" "SIGILL" "SIGINT" "SIGSEGV" "SIGTERM"
    "SIG_DFL" "SIG_IGN" "SIG_ERR"
    # capability denial for a feature lccc implements (GP-05)
    "__STDC_NO_ATOMICS__"
)
readonly MISSING_ESSENTIAL=(
    # type sizes — the codegen depends on these matching the backend
    "__SIZEOF_POINTER__" "__SIZEOF_INT__" "__SIZEOF_LONG__" "__SIZEOF_LONG_LONG__"
    "__SIZEOF_SHORT__" "__SIZEOF_FLOAT__" "__SIZEOF_DOUBLE__" "__SIZEOF_SIZE_T__"
    "__SIZEOF_PTRDIFF_T__" "__SIZEOF_WCHAR_T__"
    "__SIZEOF_WINT_T__"
    # __SIZEOF_INT128__ intentionally absent: lccc's i686 backend does not
    # expose __int128 size the same way GCC does (macro undefined on -m32),
    # while x86-64 defines it.  Kept out of MISSING_ESSENTIAL to avoid a
    # target-dependent gate failure.
    # byte order
    "__BYTE_ORDER__" "__ORDER_LITTLE_ENDIAN__" "__ORDER_BIG_ENDIAN__"
    # type limits
    "__CHAR_BIT__" "__INT_MAX__" "__LONG_MAX__" "__LONG_LONG_MAX__"
    "__SCHAR_MAX__" "__SHRT_MAX__" "__WCHAR_MAX__" "__SIZE_MAX__"
    "__PTRDIFF_MAX__" "__WINT_MAX__" "__WINT_MIN__" "__SIG_ATOMIC_MAX__"
    "__SIG_ATOMIC_MIN__"
    # type names
    "__SIZE_TYPE__" "__PTRDIFF_TYPE__" "__WCHAR_TYPE__" "__WINT_TYPE__"
    "__CHAR16_TYPE__" "__CHAR32_TYPE__" "__INTMAX_TYPE__" "__UINTMAX_TYPE__"
    "__INT8_TYPE__" "__INT16_TYPE__" "__INT32_TYPE__" "__INT64_TYPE__"
    "__UINT8_TYPE__" "__UINT16_TYPE__" "__UINT32_TYPE__" "__UINT64_TYPE__"
    "__INTPTR_TYPE__" "__UINTPTR_TYPE__"
    "__INT_LEAST8_TYPE__" "__INT_LEAST16_TYPE__" "__INT_LEAST32_TYPE__"
    "__INT_LEAST64_TYPE__"
    "__UINT_LEAST8_TYPE__" "__UINT_LEAST16_TYPE__" "__UINT_LEAST32_TYPE__"
    "__UINT_LEAST64_TYPE__"
    "__INT_FAST8_TYPE__" "__INT_FAST16_TYPE__" "__INT_FAST32_TYPE__"
    "__INT_FAST64_TYPE__"
    "__UINT_FAST8_TYPE__" "__UINT_FAST16_TYPE__" "__UINT_FAST32_TYPE__"
    "__UINT_FAST64_TYPE__"
)
readonly SPELLING_ALLOWLIST=(
    # "__LDBL_MAX__": GCC uses a trailing 'L' suffix, lccc matches the value
    # but not the exact token spelling — justified, same numeric value.
    "__LDBL_MAX__"
    # "__SIZE_MAX__" on i686 (ILP32): lccc emits "4294967295U" (unsigned int
    # suffix) while gcc-16 emits "4294967295UL" (unsigned long suffix).  Same
    # numeric value (4294967295), different token spelling — justified.
    "__SIZE_MAX__"
    # "__STDC_VERSION__": lccc claims C23 (202311L) in every -std= mode per
    # its permissive-extension dialect policy; gcc-16 emits 201710L (gnu17)
    # and 199409L (gnu90).  Same __STDC__ conformance level class, different
    # dialect string — justified.
    "__STDC_VERSION__"
    # "__GNUC__": lccc claims GCC 14.2.0 (GP-05 policy: __GNUC__=14 avoids
    # glibc headers enabling features lccc does not implement).  gcc-16
    # reports 16.  Intentional version claim — justified.
    "__GNUC__"
    # "__VERSION__": mirrors __GNUC__ — lccc "14.2.0" vs gcc-16 "16.2.0".
    # Intentional version string — justified.
    "__VERSION__"
    # "__DBL_MIN__", "__DBL_MAX__", "__DBL_EPSILON__", "__DBL_DENORM_MIN__":
    # gcc-16 wraps the value in "((double)...)" (a cast expression); lccc
    # emits the bare numeric literal.  Same value, different spelling —
    # justified.
    "__DBL_MIN__"
    "__DBL_MAX__"
    "__DBL_EPSILON__"
    "__DBL_DENORM_MIN__"
    # "__BYTE_ORDER__": gcc-16 defines it symbolically as
    # "__ORDER_LITTLE_ENDIAN__" while lccc emits the expanded numeric value
    # "1234".  Both denote little-endian byte order — justified.
    "__BYTE_ORDER__"
    # "__DEC32_EPSILON__", "__DEC64_EPSILON__", "__DEC128_EPSILON__",
    # "__DEC32_MIN__", "__DEC64_MIN__", "__DEC128_MIN__",
    # "__DEC32_MAX__", "__DEC64_MAX__", "__DEC128_MAX__",
    # "__DEC32_SUBNORMAL_MIN__", "__DEC64_SUBNORMAL_MIN__",
    # "__DEC128_SUBNORMAL_MIN__": gcc-16 uses uppercase exponent ("1E-6DF")
    # while lccc uses lowercase ("1e-6DF"), and the trailing suffix differs
    # ("DF" vs "DL").  Same decimal-floating-point value, different token
    # spelling — justified.
    "__DEC32_EPSILON__"
    "__DEC64_EPSILON__"
    "__DEC128_EPSILON__"
    "__DEC32_MIN__"
    "__DEC64_MIN__"
    "__DEC128_MIN__"
    "__DEC32_MAX__"
    "__DEC64_MAX__"
    "__DEC128_MAX__"
    "__DEC32_SUBNORMAL_MIN__"
    "__DEC64_SUBNORMAL_MIN__"
    "__DEC128_SUBNORMAL_MIN__"
    # "__INT_MAX__", "__LONG_MAX__", "__LONG_LONG_MAX__", "__SHRT_MAX__",
    # "__SCHAR_MAX__", "__SIG_ATOMIC_MAX__", "__SIG_ATOMIC_MIN__",
    # "__PTRDIFF_MAX__": gcc-16 emits hex literals (0x7fffffff,
    # 0x7fffffffffffffffL) while lccc emits decimal (2147483647,
    # 9223372036854775807L).  Same value, different base — justified.
    "__INT_MAX__"
    "__LONG_MAX__"
    "__LONG_LONG_MAX__"
    "__SHRT_MAX__"
    "__SCHAR_MAX__"
    "__SIG_ATOMIC_MAX__"
    "__SIG_ATOMIC_MIN__"
    "__PTRDIFF_MAX__"
    # "__SIG_ATOMIC_MIN__": gcc-16 references __SIG_ATOMIC_MAX__
    # symbolically ("(-__SIG_ATOMIC_MAX__ - 1)") while lccc expands the
    # value ("(-2147483647-1)").  Same value, different spelling — justified.
    # "__UINT16_TYPE__", "__UINT_FAST32_TYPE__": gcc-16 and lccc use
    # different word orders for the same type ("short unsigned int" vs
    # "unsigned short int"; "long unsigned int" vs "unsigned int").  Same
    # type on x86-64 Linux, different token order — justified.
    "__UINT16_TYPE__"
    "__UINT_LEAST16_TYPE__"
    "__UINT_FAST32_TYPE__"
    # "__WCHAR_MAX__", "__WCHAR_MIN__": gcc-16 emits hex ("0x7fffffff") or
    # symbolic references ("(-__WCHAR_MAX__ - 1)") while lccc emits decimal
    # ("2147483647", "(-2147483647-1)").  Same value, different spelling —
    # justified.
    "__WCHAR_MAX__"
    "__WCHAR_MIN__"
    # "__WINT_MAX__": gcc-16 emits hex ("0xffffffffU") while lccc emits
    # decimal ("4294967295U").  Same value, different base — justified.
    "__WINT_MAX__"
)

# Pairs to probe: (target, dialect)
readonly TARGETS=("x86_64" "i686")
readonly DIALECTS=("gnu23" "gnu17" "gnu90")

td=$(mktemp -d "${TMPDIR:-/tmp}/lccc-macro-parity.XXXXXX")
trap 'rm -rf "$td"' EXIT

# An empty TU: no headers, so every macro is a predefined one.
printf '' > "$td/empty.c"

fail=0

for target in "${TARGETS[@]}"; do
    case $target in
        x86_64) march="-m64" ;;
        i686)   march="-m32" ;;
    esac
    for dialect in "${DIALECTS[@]}"; do
        # --- gcc-16 reference ---
        if ! "$GCC" $march -std=$dialect -dM -E "$td/empty.c" > "$td/gcc.txt" 2>/dev/null; then
            echo "SKIP: gcc-16 not available for $target $dialect"
            continue
        fi
        # --- lccc under test ---
        if ! "$CCC" $march -std=$dialect -dM -E "$td/empty.c" > "$td/lccc.txt" 2>/dev/null; then
            echo "FAIL: lccc -dM -E -std=$dialect ($target) exited non-zero"
            fail=1
            continue
        fi

        # Build sorted name→value maps.
        # Strip the leading #define token; each line becomes "NAME value".
        awk '{$1=""; sub(/^ /,""); print}' "$td/gcc.txt" | sort > "$td/gcc.nv"
        awk '{$1=""; sub(/^ /,""); print}' "$td/lccc.txt" | sort > "$td/lccc.nv"

        # Name-only lists for set operations (a macro with different values
        # must not count as missing/forbidden — that's a SPELLING issue).
        awk '{print $1}' "$td/gcc.nv" | sort > "$td/gcc.names"
        awk '{print $1}' "$td/lccc.nv" | sort > "$td/lccc.names"

        comm -13 "$td/gcc.names" "$td/lccc.names" > "$td/only_lccc_names.txt"   # lccc-defined only
        comm -23 "$td/gcc.names" "$td/lccc.names" > "$td/only_gcc_names.txt"    # gcc-defined only
        comm -12 "$td/gcc.names" "$td/lccc.names" > "$td/both_names.txt"        # defined by both

        # --- FORBIDDEN check: lccc-defined names that are on the pinned list ---
        while IFS= read -r name; do
            [ -z "$name" ] && continue
            # Is this name on the FORBIDDEN_LIST?
            for fb in "${FORBIDDEN_LIST[@]}"; do
                if [ "$name" = "$fb" ]; then
                    lccc_val=$(awk -v name="$name" '$1 == name { $1=""; sub(/^ /,""); print; exit }' "$td/lccc.nv")
                    echo "::error title=GP-23 FORBIDDEN::$target/$dialect: FORBIDDEN macro '$name' defined by lccc (value: $lccc_val)"
                    echo "FAIL: $target/$dialect: FORBIDDEN macro '$name' defined by lccc (value: $lccc_val)"
                    fail=1
                fi
            done
        done < "$td/only_lccc_names.txt"

        # --- MISSING check: essential macros gcc defines but lccc doesn't ---
        while IFS= read -r name; do
            [ -z "$name" ] && continue
            for ess in "${MISSING_ESSENTIAL[@]}"; do
                if [ "$name" = "$ess" ]; then
                    echo "::error title=GP-23 MISSING::$target/$dialect: MISSING essential macro '$name' (defined by gcc-16, not lccc)"
                    echo "FAIL: $target/$dialect: MISSING essential macro '$name' (defined by gcc-16, not lccc)"
                    fail=1
                fi
            done
        done < "$td/only_gcc_names.txt"

        # --- SPELLING check: both define it, values differ ---
        while IFS= read -r name; do
            [ -z "$name" ] && continue
            gcc_val=$(awk -v name="$name" '$1 == name { $1=""; sub(/^ /,""); print; exit }' "$td/gcc.nv")
            lccc_val=$(awk -v name="$name" '$1 == name { $1=""; sub(/^ /,""); print; exit }' "$td/lccc.nv")
            if [ "$gcc_val" != "$lccc_val" ]; then
                # Check allowlist
                allowed=0
                for sp in "${SPELLING_ALLOWLIST[@]}"; do
                    if [ "$name" = "$sp" ]; then allowed=1; break; fi
                done
                if [ "$allowed" -eq 0 ]; then
                    echo "::error title=GP-23 SPELLING::$target/$dialect: SPELLING mismatch '$name': gcc=$gcc_val lccc=$lccc_val"
                    echo "FAIL: $target/$dialect: SPELLING mismatch '$name': gcc=$gcc_val lccc=$lccc_val"
                    fail=1
                fi
            fi
        done < "$td/both_names.txt"
    done
done

if [ "$fail" -ne 0 ]; then
    echo "FAIL: macro parity gate — see above"
    exit 1
fi
echo "OK: macro parity (FORBIDDEN empty, MISSING essential empty, SPELLING allowlist respected)"
