#!/usr/bin/env bash
# A14-13: ext-vector logical lanes, padded object layout, initialization,
# multidimensional array strides, and emitted ELF symbol sizes/alignments.
set -euo pipefail
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
ccc=${CCC:-$root/target/fastbuild/lccc}
clang_bin=${CLANG:-clang-19}
gcc_bin=${GCC:-gcc}
readelf_bin=${READELF:-readelf}
source "$root/tests/regression/i386_exec.sh"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
src=$root/tests/regression/ext_vector_layout.c

[ -x "$ccc" ] || { echo "FAIL: compiler not built: $ccc" >&2; exit 1; }
command -v "$readelf_bin" >/dev/null 2>&1 || {
    echo "FAIL: readelf not available for emitted-object layout checks" >&2
    exit 1
}
clang_available=1
if ! command -v "$clang_bin" >/dev/null 2>&1; then
    clang_available=0
    if [[ ${LCCC_REQUIRE_CLANG:-0} == 1 ]]; then
        echo "FAIL: required Clang oracle not found: $clang_bin" >&2
        exit 1
    fi
    echo "SKIP: Clang layout oracle not found: $clang_bin"
fi

# The emitted object must have the C object's size, not the IR pointer carrier
# size. Alignment is checked from the ELF symbol value as well as by the
# translation unit's _Alignof assertions.
check_symbol_alignment() {
    local obj=$1 symbol=$2 expected_align=$3 value
    value=$("$readelf_bin" -sW "$obj" | awk -v name="$symbol" \
        '$4 == "OBJECT" && $8 == name { print $2; exit }')
    if [[ -z $value ]]; then
        echo "FAIL: $symbol missing from $obj" >&2
        "$readelf_bin" -sW "$obj" | grep -F "$symbol" >&2 || true
        exit 1
    fi
    # readelf prints a zero-padded hexadecimal section-relative symbol value.
    if (( (16#$value) % expected_align != 0 )); then
        echo "FAIL: $symbol address 0x$value is not $expected_align-byte aligned in $obj" >&2
        exit 1
    fi
}

check_symbol_layout() {
    local obj=$1 symbol=$2 expected_size=$3 expected_align=$4 actual_size
    actual_size=$("$readelf_bin" -sW "$obj" | awk -v name="$symbol" \
        '$4 == "OBJECT" && $8 == name { print $3; exit }')
    if [[ -z $actual_size ]]; then
        echo "FAIL: $symbol missing from $obj" >&2
        "$readelf_bin" -sW "$obj" | grep -F "$symbol" >&2 || true
        exit 1
    fi
    if [[ $actual_size != "$expected_size" ]]; then
        echo "FAIL: $symbol size is $actual_size, expected $expected_size in $obj" >&2
        exit 1
    fi
    check_symbol_alignment "$obj" "$symbol" "$expected_align"
}

check_objects() {
    local compiler=$1 object=$2
    local -a expected=(
        'global_bool4 1 1'
        'global_bool9 2 2'
        'global_bool17 4 4'
        'global_u32x3 16 16'
        'global_u32x3_pair 32 16'
        'global_bool4_pair 2 1'
        'global_u8x3_pair 8 4'
        'global_u32x3_matrix 64 16'
        'global_u32x3_designated_matrix 64 16'
        'global_u32x3_compounds 32 16'
        'global_ld3 64 64'
        'global_vector_fields 32 16'
    )
    local spec name size align actual_size
    for spec in "${expected[@]}"; do
        read -r name size align <<<"$spec"
        if [[ $compiler == Clang && $name == global_ld3 ]]; then
            # Clang 19.1.7 and ICX 2026.2 report sizeof/alignof as 64/64,
            # but emit a 32-byte ELF object for this x87 vector initializer:
            # LLVM packs `<3 x x86_fp80>` at the 80-bit element size (30
            # bytes, allocated as 32) while the C object is 64. lccc matches
            # the 10-byte lane stride (ABI-EXTVEC-LD-1) but emits the full
            # C-object size, so only the alignment is compared for Clang.
            check_symbol_alignment "$object" "$name" "$align"
            actual_size=$("$readelf_bin" -sW "$object" | awk -v name="$name" \
                '$4 == "OBJECT" && $8 == name { print $3; exit }')
            if [[ $actual_size != "$size" ]]; then
                echo "NOTE: Clang global_ld3 ELF size is ${actual_size:-missing} (LLVM alloc size of <3 x x86_fp80>), sizeof(ld3) is 64; size comparison excluded"
            else
                echo "PASS: Clang global_ld3 ELF size matches sizeof(ld3)"
            fi
        else
            check_symbol_layout "$object" "$name" "$size" "$align"
        fi
    done
    echo "PASS: $compiler emitted object sizes and alignments (except noted oracle limitation)"
}

for bits in 64 32; do
    if [[ $bits == 32 ]] && ! i386_run_or_skip "$gcc_bin"; then
        if [[ ${LCCC_REQUIRE_I686:-0} == 1 ]]; then exit 1; fi
        continue
    fi

    # LCCC: executable behavior at the project's fast test optimization.
    lccc_exe=$tmp/lccc-layout-$bits
    if ! "$ccc" -m"$bits" -O1 "$src" -o "$lccc_exe" >"$tmp/lccc-$bits.log" 2>&1; then
        echo "FAIL: ext-vector layout compile -m$bits -O1" >&2
        cat "$tmp/lccc-$bits.log" >&2
        exit 1
    fi
    rc=0
    "$lccc_exe" || rc=$?
    if [[ $rc -ne 0 ]]; then
        echo "FAIL: ext-vector layout/runtime -m$bits -O1 (exit $rc)" >&2
        exit 1
    fi
    echo "PASS: ext-vector layout/runtime -m$bits -O1"

    lccc_obj=$tmp/lccc-layout-$bits.o
    "$ccc" -m"$bits" -O1 -c "$src" -o "$lccc_obj"
    check_objects LCCC "$lccc_obj"

    # This volatile probe forces actual reads of every lane from the global
    # object, avoiding a constant-folded initializer check. Clang/ICX are not
    # used as size or byte-layout oracles for this known-broken x87-vector case.
    ld3_src=$root/tests/regression/ext_vector_ld3_global.c
    ld3_exe=$tmp/lccc-ld3-global-$bits
    if ! "$ccc" -m"$bits" -O1 "$ld3_src" -o "$ld3_exe" >"$tmp/lccc-ld3-$bits.log" 2>&1; then
        echo "FAIL: long-double ext-vector global compile -m$bits -O1" >&2
        cat "$tmp/lccc-ld3-$bits.log" >&2
        exit 1
    fi
    rc=0
    "$ld3_exe" || rc=$?
    if [[ $rc -ne 0 ]]; then
        echo "FAIL: long-double ext-vector global memory/runtime -m$bits -O1 (exit $rc)" >&2
        exit 1
    fi
    ld3_obj=$tmp/lccc-ld3-global-$bits.o
    "$ccc" -m"$bits" -O1 -c "$ld3_src" -o "$ld3_obj"
    check_symbol_layout "$ld3_obj" global_ld3 64 64
    echo "PASS: long-double ext-vector global memory/layout -m$bits -O1"

    # Vector temporaries: an alloca followed by lane stores only defines the
    # whole object when no lane store is a read-modify-write (bit-packed bool
    # lanes) and the lanes cover every byte (no padding). Clearing outside those
    # cases is dead stores, which is measurable IR bloat; clearing inside them
    # is required, and so is clearing the C-mandated zero tail of a short
    # initializer list. Both directions are pinned by ext_vector_temp_init.c.
    init_src=$root/tests/regression/ext_vector_temp_init.c
    for opt in -O0 -O1 -O2; do
        init_exe=$tmp/lccc-temp-init-$bits$opt
        if ! "$ccc" -m"$bits" $opt -DLCCC_PADDING_DETERMINISTIC=1 "$init_src" -o "$init_exe" \
            >"$tmp/init-$bits$opt.log" 2>&1; then
            echo "FAIL: ext-vector temporary init compile -m$bits $opt" >&2
            cat "$tmp/init-$bits$opt.log" >&2
            exit 1
        fi
        rc=0
        "$init_exe" >"$tmp/init-$bits$opt.out" || rc=$?
        if [[ $rc -ne 0 ]]; then
            echo "FAIL: ext-vector temporary initialization -m$bits $opt (exit $rc)" >&2
            cat "$tmp/init-$bits$opt.out" >&2
            exit 1
        fi
        echo "PASS: ext-vector temporary initialization (lanes + padding) -m$bits $opt"
    done

    if [[ $clang_available == 1 ]]; then
        # Clang is the semantic/layout oracle; use O0 to avoid known Clang-19
        # i386 optimizer failures. Non-PIE is needed only for sandboxed i386
        # execution, not for compilation to an object file.
        clang_exe=$tmp/clang-layout-$bits
        clang_flags=()
        if [[ $bits == 32 ]]; then clang_flags+=(-no-pie); fi
        if ! "$clang_bin" -m"$bits" -O0 "${clang_flags[@]}" "$src" -o "$clang_exe" \
            >"$tmp/clang-$bits.log" 2>&1; then
            echo "FAIL: Clang ext-vector layout oracle compile -m$bits -O0" >&2
            cat "$tmp/clang-$bits.log" >&2
            exit 1
        fi
        rc=0
        "$clang_exe" || rc=$?
        if [[ $rc -ne 0 ]]; then
            echo "FAIL: Clang ext-vector layout oracle -m$bits -O0 (exit $rc)" >&2
            exit 1
        fi
        echo "PASS: Clang ext-vector layout/runtime oracle -m$bits -O0"

        # Portable half of the temporary-init test: logical lanes only, because
        # Clang leaves the padding of an automatic ext-vector indeterminate
        # (measured on i386 -O0: 0xf6/0xf7). Build LCCC and Clang the same way
        # and require byte-identical results at every LCCC optimization level.
        clang_init_exe=$tmp/clang-temp-init-$bits
        if ! "$clang_bin" -m"$bits" -O0 "${clang_flags[@]}" "$init_src" -o "$clang_init_exe" \
            >"$tmp/clang-init-$bits.log" 2>&1; then
            echo "FAIL: Clang ext-vector temporary init oracle compile -m$bits -O0" >&2
            cat "$tmp/clang-init-$bits.log" >&2
            exit 1
        fi
        rc=0
        "$clang_init_exe" >"$tmp/clang-init-$bits.out" || rc=$?
        if [[ $rc -ne 0 ]]; then
            echo "FAIL: Clang ext-vector temporary init oracle -m$bits -O0 (exit $rc)" >&2
            cat "$tmp/clang-init-$bits.out" >&2
            exit 1
        fi
        for opt in -O0 -O1 -O2; do
            lccc_init_exe=$tmp/lccc-temp-init-portable-$bits$opt
            if ! "$ccc" -m"$bits" $opt "$init_src" -o "$lccc_init_exe" \
                >"$tmp/init-p-$bits$opt.log" 2>&1; then
                echo "FAIL: ext-vector temporary init compile -m$bits $opt" >&2
                cat "$tmp/init-p-$bits$opt.log" >&2
                exit 1
            fi
            rc=0
            "$lccc_init_exe" >"$tmp/init-p-$bits$opt.out" || rc=$?
            if [[ $rc -ne 0 ]]; then
                echo "FAIL: ext-vector temporary initialization -m$bits $opt (exit $rc)" >&2
                cat "$tmp/init-p-$bits$opt.out" >&2
                exit 1
            fi
            if ! diff -u "$tmp/clang-init-$bits.out" "$tmp/init-p-$bits$opt.out" >"$tmp/init-p-$bits$opt.diff"; then
                echo "FAIL: ext-vector temporary initialization -m$bits $opt differs from the Clang oracle" >&2
                cat "$tmp/init-p-$bits$opt.diff" >&2
                exit 1
            fi
            echo "PASS: ext-vector temporary initialization matches Clang -m$bits $opt"
        done

        clang_obj=$tmp/clang-layout-$bits.o
        "$clang_bin" -m"$bits" -O0 -c "$src" -o "$clang_obj"
        check_objects Clang "$clang_obj"
    fi
done
