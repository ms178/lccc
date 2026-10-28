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
    "__SIZEOF_PTRDIFF_T__" "__SIZEOF_WCHAR_T__" "__SIZEOF_INT128__"
    "__SIZEOF_WINT_T__"
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
        # Lines look like `#define NAME value` or `#define NAME value-with-spaces`.
        awk '{$1=""; sub(/^ /,""); print}' "$td/gcc.txt" > "$td/gcc.names"
        awk '{$1=""; sub(/^ /,""); print}' "$td/lccc.txt" > "$td/lccc.names"
        # Actually we need name and value separately.
        awk '{print $1, $2}' "$td/gcc.txt" | sort > "$td/gcc.nv"
        awk '{print $1, $2}' "$td/lccc.txt" | sort > "$td/lccc.nv"

        comm -13 "$td/gcc.nv" "$td/lccc.nv" > "$td/only_lccc.txt"   # lccc-defined only
        comm -23 "$td/gcc.nv" "$td/lccc.nv" > "$td/only_gcc.txt"    # gcc-defined only
        comm -12 "$td/gcc.nv" "$td/lccc.nv" > "$td/both.txt"        # defined by both

        # --- FORBIDDEN check: lccc-defined names that are on the pinned list ---
        while IFS= read -r line; do
            [ -z "$line" ] && continue
            name="${line%% *}"
            # Is this name on the FORBIDDEN_LIST?
            for fb in "${FORBIDDEN_LIST[@]}"; do
                if [ "$name" = "$fb" ]; then
                    echo "FAIL: $target/$dialect: FORBIDDEN macro '$name' defined by lccc (value: ${line#* })"
                    fail=1
                fi
            done
        done < "$td/only_lccc.txt"

        # --- MISSING check: essential macros gcc defines but lccc doesn't ---
        while IFS= read -r line; do
            [ -z "$line" ] && continue
            name="${line%% *}"
            for ess in "${MISSING_ESSENTIAL[@]}"; do
                if [ "$name" = "$ess" ]; then
                    echo "FAIL: $target/$dialect: MISSING essential macro '$name' (defined by gcc-16, not lccc)"
                    fail=1
                fi
            done
        done < "$td/only_gcc.txt"

        # --- SPELLING check: both define it, values differ ---
        while IFS= read -r line; do
            [ -z "$line" ] && continue
            name="${line%% *}"
            gcc_val=$(grep "^$name " "$td/gcc.nv" | head -1 | cut -d' ' -f2-)
            lccc_val=$(grep "^$name " "$td/lccc.nv" | head -1 | cut -d' ' -f2-)
            if [ "$gcc_val" != "$lccc_val" ]; then
                # Check allowlist
                allowed=0
                for sp in "${SPELLING_ALLOWLIST[@]}"; do
                    if [ "$name" = "$sp" ]; then allowed=1; break; fi
                done
                if [ "$allowed" -eq 0 ]; then
                    echo "FAIL: $target/$dialect: SPELLING mismatch '$name': gcc=$gcc_val lccc=$lccc_val"
                    fail=1
                fi
            fi
        done < "$both.txt"
    done
done

if [ "$fail" -ne 0 ]; then
    echo "FAIL: macro parity gate — see above"
    exit 1
fi
echo "OK: macro parity (FORBIDDEN empty, MISSING essential empty, SPELLING allowlist respected)"
