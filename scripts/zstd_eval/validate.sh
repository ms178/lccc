#!/bin/sh
# Full zstd validation of one compiler (inputs from setup.sh):
#   1. fresh copy of the zstd 1.5.7 tree, CLI built with -O2 (no asm/libs)
#   2. census: FORCE_INLINE template names that survived as text symbols
#      (want 0; GCC and Clang outline none of them at -O2)
#   3. round trip vs the reference CLI (cross-decoding, 4 levels, 3 inputs)
#   4. zstd's own fuzzer (-s7) for the given seconds (default 150; 0 = skip)
# usage: validate.sh <compiler> <out-binary> [fuzz-seconds]
set -u
L=$1; OUT=$2; FUZZ=${3:-150}
D=${ZSTD_EVAL_DIR:-${XDG_CACHE_HOME:-$HOME/.cache}/lccc-zstd-eval}
HERE=$(cd "$(dirname "$0")" && pwd)
[ -d "$D/zstd-1.5.7" ] && [ -x "$D/zstd-ref" ] || { echo "run setup.sh first"; exit 2; }
W=$D/work.$$
rc=0
rm -rf "$W"; cp -r "$D/zstd-1.5.7" "$W" || exit 2
make -s -j1 -C "$W/programs" zstd CC="$L" ZSTD_NO_ASM=1 HAVE_ZLIB=0 HAVE_LZMA=0 \
    HAVE_LZ4=0 MOREFLAGS="-O2" > "$W/build.log" 2>&1 || { echo "BUILD FAIL"; tail -20 "$W/build.log"; exit 1; }
cp "$W/programs/zstd" "$OUT"
echo "size $(stat -c %s "$OUT")"
# Text symbols only: a function-local static such as
# `BIT_reloadDStream.zeroFilled.0` (rodata) is not an outlined copy.
nm "$OUT" | awk '$2 ~ /^[tT]$/ {print $3}' | sed 's/\..*//' | sort -u > "$W/syms.txt"
n=$(grep -cxFf "$HERE/force_inline_template_names.txt" "$W/syms.txt")
echo "census outlined_force_inline=$n"; [ "$n" -eq 0 ] || rc=1
sh "$HERE/roundtrip.sh" "$OUT" "$D/zstd-ref" "$D/src.tar" "$D/rand.bin" "$D/input.bin" || rc=1
if [ "$FUZZ" -gt 0 ]; then
    make -s -j1 -C "$W/tests" fuzzer CC="$L" ZSTD_NO_ASM=1 MOREFLAGS="-O2" > "$W/fb.log" 2>&1 \
        || { echo "FUZZER BUILD FAIL"; tail -20 "$W/fb.log"; rc=1; }
    if [ -x "$W/tests/fuzzer" ]; then
        (cd "$W/tests" && ./fuzzer -T"${FUZZ}"s -s7 > "$W/fz.log" 2>&1); f=$?
        tr '\r' '\n' < "$W/fz.log" | grep -v '^$' | tail -n 2
        echo "fuzzer exit=$f"; [ $f -eq 0 ] || rc=1
    fi
fi
rm -rf "$W"
echo "VALIDATE rc=$rc"; exit $rc
