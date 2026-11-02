#!/bin/sh
# Prepare every zstd evaluation input in $ZSTD_EVAL_DIR (default
# ${XDG_CACHE_HOME:-$HOME/.cache}/lccc-zstd-eval). Idempotent.
#   zstd-1.5.7/   pristine source (sha256-checked release tarball)
#   input.bin     1.5 MiB benchmark input: the first 1572864 bytes of the
#                 tree's sorted lib/programs/tests C sources (sha256 pinned)
#   src.tar       round-trip input: the lib/ tree as a tar
#   rand.bin      round-trip input: 1 MiB from Python's Random(7)
#   zstd-ref      GCC -O2 reference CLI (same flags validate.sh gives lccc)
# usage: setup.sh   (env: ZSTD_EVAL_DIR, ZSTD_REF_CC=gcc-16)
set -eu
D=${ZSTD_EVAL_DIR:-${XDG_CACHE_HOME:-$HOME/.cache}/lccc-zstd-eval}
HERE=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$D"
T=$D/zstd-1.5.7.tar.gz
SUM=eb33e51f49a15e023950cd7825ca74a4a2b43db8354825ac24fc1b7ee09e6fa3
[ -f "$T" ] || curl -fsSL -o "$T" https://github.com/facebook/zstd/releases/download/v1.5.7/zstd-1.5.7.tar.gz
echo "$SUM  $T" | sha256sum -c - >/dev/null
[ -d "$D/zstd-1.5.7" ] || tar -xzf "$T" -C "$D"
# No pipeline consumer that stops early: concatenate, then truncate the file.
(cd "$D/zstd-1.5.7" && find lib programs tests -name '*.[ch]' | LC_ALL=C sort | xargs cat) > "$D/input.full"
head -c 1572864 "$D/input.full" > "$D/input.bin"; rm -f "$D/input.full"
echo "ee05dd84a3b957ce879b9795773c3d3090c774e951fc56b0454d9c4c3740a327  $D/input.bin" | sha256sum -c - >/dev/null
tar -cf "$D/src.tar" -C "$D" zstd-1.5.7/lib
python3 -c "import random,sys; r=random.Random(7); sys.stdout.buffer.write(bytes(r.getrandbits(8) for _ in range(1<<20)))" > "$D/rand.bin"
if [ ! -x "$D/zstd-ref" ]; then
  W=$D/ref.$$; rm -rf "$W"; cp -r "$D/zstd-1.5.7" "$W"
  make -s -j2 -C "$W/programs" zstd CC="${ZSTD_REF_CC:-gcc-16}" ZSTD_NO_ASM=1 HAVE_ZLIB=0 \
      HAVE_LZMA=0 HAVE_LZ4=0 MOREFLAGS=-O2 >/dev/null
  cp "$W/programs/zstd" "$D/zstd-ref"; rm -rf "$W"
fi
cp "$HERE/force_inline_template_names.txt" "$D/fit.txt"
echo "zstd evaluation inputs ready in $D"
