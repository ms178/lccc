#!/usr/bin/env bash
# Four-ISA C/SSA/codegen regression plus freestanding runtime oracles on
# i686 (when supported) and AArch64/RISC-V 64 (when QEMU-user is available).
set -euo pipefail
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
BIN=${LCCC_BIN_DIR:-$ROOT/target/fastbuild}
CLANG=${CLANG:-clang-23}
GCC=${GCC:-gcc-16}
SRC="$ROOT/tests/workloads/sqlite-3.54.1/ptr_delta_phi.c"
NEGATIVE="$ROOT/tests/workloads/sqlite-3.54.1/ptr_delta_phi_negative.c"
NEGATIVE_ADD="$ROOT/tests/workloads/sqlite-3.54.1/ptr_delta_phi_negative_add.c"
for tool in python3 timeout readelf "$CLANG" "$GCC" "$BIN/lccc" "$BIN/lccc-i686" \
            "$BIN/lccc-arm" "$BIN/lccc-riscv"; do
    command -v "$tool" > /dev/null || { echo "missing compiler/tool: $tool" >&2; exit 2; }
done
if [[ -n ${PTR_DELTA_CROSS_ARTIFACT_DIR:-} ]]; then
    work=$PTR_DELTA_CROSS_ARTIFACT_DIR
    mkdir -p "$work"
else
    work=$(mktemp -d)
    trap 'rm -rf -- "$work"' EXIT
fi
# Extract the ONE upstream-derived function so cross targets need no host
# stdint/stdio headers or maintained second copy of the decoder.
python3 - "$SRC" "$work/decoder.c" <<'PY'
import pathlib, sys
source = pathlib.Path(sys.argv[1]).read_text()
needle = '__attribute__((noinline))\nu8 sqlite_varint_delta('
start = source.index(needle)
brace = source.index('{', start)
depth = 0
for end in range(brace, len(source)):
    if source[end] == '{':
        depth += 1
    elif source[end] == '}':
        depth -= 1
        if depth == 0:
            break
else:
    raise SystemExit('missing closing brace in decoder fixture')
pathlib.Path(sys.argv[2]).write_text(
    'typedef unsigned char u8;\ntypedef unsigned long long u64;\n'
    + source[start:end + 1] + '\n'
)
PY
export CCC_VERIFY_IR=abort CCC_VALIDATE_SSA=1
flags=(-O2 -Wall -Wextra -Werror)
for arch in x86_64 i686 aarch64 riscv64; do
    case "$arch" in
        x86_64) cc="$BIN/lccc"; ref=("$GCC" -march=x86-64-v3); archflags=(-march=x86-64-v3) ;;
        i686) cc="$BIN/lccc-i686"; ref=("$GCC" -m32); archflags=() ;;
        aarch64) cc="$BIN/lccc-arm"; ref=("$CLANG" --target=aarch64-linux-gnu); archflags=() ;;
        riscv64) cc="$BIN/lccc-riscv"; ref=("$CLANG" --target=riscv64-linux-gnu -march=rv64gc -mabi=lp64d); archflags=() ;;
    esac
    timeout --kill-after=5s 120 "$cc" "${flags[@]}" "${archflags[@]}" \
        -S "$work/decoder.c" -o "$work/$arch-on.s"
    CCC_DISABLE_PASSES=ptr_delta_phi timeout --kill-after=5s 120 \
        "$cc" "${flags[@]}" "${archflags[@]}" -S "$work/decoder.c" -o "$work/$arch-off.s"
    timeout --kill-after=5s 120 "${ref[@]}" "${flags[@]}" \
        -S "$work/decoder.c" -o "$work/$arch-ref.s"
    for key in on off ref; do
        grep -q 'sqlite_varint_delta:' "$work/$arch-$key.s" || {
            echo "missing decoder in $arch/$key assembly" >&2; exit 1;
        }
    done
    # Assert the exact mechanism rather than calling any assembly difference a
    # win: the disabled path subtracts two pointer registers at the join; the
    # enabled path selects a constant delta and has NO register subtraction.
    # Ignore i686's stack-adjust `subl $N, %esp` (an immediate, not pointer).
    case "$arch" in
        x86_64|i686) pointer_sub='^[[:space:]]+sub[lq][[:space:]]+%[a-z0-9]+,[[:space:]]+%[a-z0-9]+' ;;
        aarch64) pointer_sub='^[[:space:]]+sub[[:space:]]+x[0-9]+,[[:space:]]+x[0-9]+,[[:space:]]+x[0-9]+' ;;
        riscv64) pointer_sub='^[[:space:]]+sub[[:space:]]+[a-z][a-z0-9]*,[[:space:]]+[a-z][a-z0-9]*,[[:space:]]+[a-z][a-z0-9]*' ;;
    esac
    if cmp -s "$work/$arch-on.s" "$work/$arch-off.s" ||
        ! grep -Eq "$pointer_sub" "$work/$arch-off.s" ||
        grep -Eq "$pointer_sub" "$work/$arch-on.s"; then
        echo "$arch: varint pass toggle did not remove the pointer subtraction" >&2
        exit 1
    fi
    # Two encodings of a backward step: Sub(base, 7) and GEP/Add(base, -7).
    # Both shrank x86 assembly yet regressed in separate 31-pair host screens.
    # Require the *function* to remain identical under the pass toggle and
    # retain its pointer subtraction on every supported target.
    for shape in negative negative-add; do
        source=$NEGATIVE
        symbol=guarded_byte_delta
        if [[ $shape == negative-add ]]; then
            source=$NEGATIVE_ADD
            symbol=guarded_negative_add_delta
        fi
        timeout --kill-after=5s 120 "$cc" "${flags[@]}" "${archflags[@]}" \
            -S "$source" -o "$work/$arch-$shape-on.s"
        CCC_DISABLE_PASSES=ptr_delta_phi timeout --kill-after=5s 120 \
            "$cc" "${flags[@]}" "${archflags[@]}" -S "$source" -o "$work/$arch-$shape-off.s"
        timeout --kill-after=5s 120 "${ref[@]}" "${flags[@]}" \
            -S "$source" -o "$work/$arch-$shape-ref.s"
        for mode in on off ref; do
            grep -q "^$symbol:" "$work/$arch-$shape-$mode.s" || {
                echo "missing $shape function in $arch/$mode assembly" >&2; exit 1;
            }
            sed -n "/^$symbol:/,/^\\.size[[:space:]]*$symbol,/p" \
                "$work/$arch-$shape-$mode.s" > "$work/$arch-$shape-$mode.func"
        done
        if ! cmp -s "$work/$arch-$shape-on.func" "$work/$arch-$shape-off.func" ||
            ! grep -Eq "$pointer_sub" "$work/$arch-$shape-on.func"; then
            echo "$arch: $shape negative control changed under the pass toggle" >&2
            exit 1
        fi
    done
    echo "$arch: varint fold plus both unchanged backward controls, C/SSA/warning gates: PASS"
done

# A normal Linux/x86_64 host can often execute 32-bit static ELF without a
# multilib C runtime. Probe that ability before reporting the additional i686
# oracle result; the other targets remain compile-only without an emulator.
if [[ $(uname -s) == Linux && $(uname -m) == x86_64 ]] &&
    command -v as >/dev/null && command -v ld >/dev/null; then
    cat > "$work/i686-probe.S" <<'ASM'
.globl _start
_start:
    movl $1, %eax
    xorl %ebx, %ebx
    int $0x80
.section .note.GNU-stack,"",@progbits
ASM
    if as --32 "$work/i686-probe.S" -o "$work/i686-probe.o" &&
        ld -m elf_i386 -o "$work/i686-probe" "$work/i686-probe.o" &&
        timeout --kill-after=5s 5 "$work/i686-probe"; then
        cat > "$work/i686-entry.S" <<'ASM'
.globl _start
_start:
    call main
    movl %eax, %ebx
    movl $1, %eax
    int $0x80
.section .note.GNU-stack,"",@progbits
ASM
        as --32 "$work/i686-entry.S" -o "$work/i686-entry.o"
        oracle="$ROOT/tests/workloads/sqlite-3.54.1/ptr_delta_phi_freestanding_oracle.c"
        opts=(-std=c11 -O2 -Wall -Wextra -Werror -fno-pic -fno-pie)
        "$GCC" -m32 "${opts[@]}" -c "$oracle" -o "$work/i686-oracle.o"
        for variant in gcc clang lccc lccc-off; do
            case "$variant" in
                gcc) compiler=("$GCC" -m32); environment=() ;;
                clang) compiler=("$CLANG" --target=i686-linux-gnu); environment=() ;;
                lccc) compiler=("$BIN/lccc-i686"); environment=() ;;
                lccc-off) compiler=("$BIN/lccc-i686"); environment=(env CCC_DISABLE_PASSES=ptr_delta_phi) ;;
            esac
            "${environment[@]}" "${compiler[@]}" "${opts[@]}" -c \
                "$work/decoder.c" -o "$work/i686-$variant-decoder.o"
            "${environment[@]}" "${compiler[@]}" "${opts[@]}" -c \
                "$NEGATIVE" -o "$work/i686-$variant-negative.o"
            "${environment[@]}" "${compiler[@]}" "${opts[@]}" -c \
                "$NEGATIVE_ADD" -o "$work/i686-$variant-negative-add.o"
            ld -m elf_i386 -o "$work/i686-$variant" "$work/i686-entry.o" \
                "$work/i686-oracle.o" "$work/i686-$variant-decoder.o" \
                "$work/i686-$variant-negative.o" "$work/i686-$variant-negative-add.o"
            timeout --kill-after=5s 30 "$work/i686-$variant" || {
                rc=$?; echo "i686: $variant failed varint/backward-step GCC oracle (exit $rc)" >&2; exit 1;
            }
        done
        echo 'i686: 90,000 varints and 100,000 deltas x four compilers/modes: PASS (freestanding IA32)'
    else
        echo 'i686: runtime unavailable on this host; cross C/SSA/codegen gate passed'
    fi
else
    echo 'i686: runtime unavailable on this host; cross C/SSA/codegen gate passed'
fi

# With pinned QEMU user-mode binaries, execute the SAME independent oracle on
# the remaining targets. Clang cross-compiles the reference translation unit;
# LCCC compiles the decoder separately, assembles a libc-free Linux _start,
# and links statically. A Clang-compiled decoder is an extra reference arm.
for arch in aarch64 riscv64; do
    qemu="qemu-$arch"
    if ! command -v "$qemu" >/dev/null; then
        echo "$arch: QEMU unavailable; compile/codegen-only gate passed (no runtime claim)"
        continue
    fi
    case "$arch" in
        aarch64)
            cc="$BIN/lccc-arm"
            triple=aarch64-linux-gnu
            machine=AArch64
            cat > "$work/aarch64-entry.s" <<'ASM'
.text
.globl _start
_start:
    bl main
    mov x8, #93
    svc #0
ASM
            ;;
        riscv64)
            cc="$BIN/lccc-riscv"
            triple=riscv64-linux-gnu
            machine='RISC-V'
            cat > "$work/riscv64-entry.s" <<'ASM'
.text
.globl _start
_start:
    call main
    li a7, 93
    ecall
ASM
            ;;
    esac
    oracle="$ROOT/tests/workloads/sqlite-3.54.1/ptr_delta_phi_freestanding_oracle.c"
    opts=(-std=c11 -O2 -Wall -Wextra -Werror -fno-pic -fno-pie)
    "$cc" -c "$work/$arch-entry.s" -o "$work/$arch-entry.o"
    "$CLANG" --target="$triple" "${opts[@]}" -c "$oracle" -o "$work/$arch-oracle.o"
    for variant in clang lccc lccc-off; do
        case "$variant" in
            clang) compiler=("$CLANG" --target="$triple"); environment=() ;;
            lccc) compiler=("$cc"); environment=() ;;
            lccc-off) compiler=("$cc"); environment=(env CCC_DISABLE_PASSES=ptr_delta_phi) ;;
        esac
        "${environment[@]}" "${compiler[@]}" "${opts[@]}" -c \
            "$work/decoder.c" -o "$work/$arch-$variant-decoder.o"
        "${environment[@]}" "${compiler[@]}" "${opts[@]}" -c \
            "$NEGATIVE" -o "$work/$arch-$variant-negative.o"
        "${environment[@]}" "${compiler[@]}" "${opts[@]}" -c \
            "$NEGATIVE_ADD" -o "$work/$arch-$variant-negative-add.o"
        "$cc" -nostdlib -static "$work/$arch-entry.o" "$work/$arch-oracle.o" \
            "$work/$arch-$variant-decoder.o" "$work/$arch-$variant-negative.o" \
            "$work/$arch-$variant-negative-add.o" -o "$work/$arch-$variant"
        readelf -h "$work/$arch-$variant" | grep -qE "Machine: *$machine$" || {
            echo "$arch/$variant: incorrect ELF machine" >&2; exit 1;
        }
        if readelf -l "$work/$arch-$variant" | grep -q INTERP; then
            echo "$arch/$variant: not freestanding (dynamic loader)" >&2; exit 1;
        fi
        timeout --kill-after=5s 30 "$qemu" "$work/$arch-$variant" || {
            rc=$?; echo "$arch/$variant failed varint/backward-step Clang oracle (exit $rc)" >&2; exit 1;
        }
    done
    echo "$arch: 90,000 varints and 100,000 deltas x Clang/LCCC-on/LCCC-off: PASS (static QEMU)"
done
