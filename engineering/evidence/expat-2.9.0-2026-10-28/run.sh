#!/usr/bin/env bash
# Reproduce the manual upstream-source experiment without installing packages.
# Usage: GCC=gcc-16 LCCC=/path/to/lccc CLANG=clang ./run.sh EXPAT_SOURCE OUT_DIR
set -euo pipefail

if [[ $# -ne 2 ]]; then
    echo "usage: $0 EXPAT_SOURCE OUT_DIR" >&2
    exit 2
fi
here=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
src=$(realpath -e -- "$1")
out=$(realpath -m -- "$2")
home=$(realpath -e -- "$HOME")
case "$out/" in "$home/"*|/tmp/*) ;; *) echo "output must be under HOME or /tmp" >&2; exit 2;; esac
case "$out/" in "$src/"*) echo "output must not be inside the source tree" >&2; exit 2;; esac
mkdir -p -- "$out"
out=$(realpath -e -- "$out")
case "$out/" in "$home/"*|/tmp/*) ;; *) echo "output escaped the allowed prefix" >&2; exit 2;; esac

GCC=${GCC:-gcc-16}
LCCC=${LCCC:-lccc}
CLANG=${CLANG:-clang}
for cc in "$GCC" "$LCCC" "$CLANG"; do
    command -v "$cc" >/dev/null || { echo "compiler not found: $cc" >&2; exit 2; }
done
# Relative compiler paths must survive the working-directory changes below.
GCC=$(realpath -e -- "$(command -v "$GCC")")
LCCC=$(realpath -e -- "$(command -v "$LCCC")")
CLANG=$(realpath -e -- "$(command -v "$CLANG")")

grep -q '^#  define XML_MICRO_VERSION 0$' "$src/lib/expat.h"
(
    cd "$src"
    sha256sum -c <<'EOF'
899afeaddf424a6f4bf9a0a77159248b485bf5c072475d547ef16c2942a86dd5  lib/xmlparse.c
44baab2a4970749f6f29992bbbc1b3c62239ef9ec2a20de96332ef4659b9e32d  lib/xmltok.c
b7c4d41782fd78a50ba080f67060372cd035f8c3a44a023b4b74facf1b9cc62d  tests/runtests.c
d4d351e5eba8869a6de8ae5ec82b747010582b0497e0eb414e1ca5d6f579827d  tests/benchmark/benchmark.c
EOF
)
printf 'GCC %s\n' "$("$GCC" -dumpfullversion)"
"$CLANG" --version | head -1
"$LCCC" --version | head -1
sha256sum "$LCCC" "$here/expat_config.h"
mkdir -p "$out"/{gcc-lib,lccc-lib,clang-lib,gcc-tests,lccc-tests}
flags=(-O2 -std=gnu11 -DXML_TESTING -DXML_STATIC -I"$here" -I"$src/lib" -I"$src/tests")
lib_sources=(xcs.c xmlparse.c xmltok.c xmlrole.c random_getrandom.c)
test_sources=(acc_tests.c alloc_tests.c basic_tests.c chardata.c common.c dummy.c handlers.c hash_tests.c memcheck.c minicheck.c misc_tests.c ns_tests.c nsalloc_tests.c props_tests.c runtests.c structdata.c xcs_tests.c)
for name in gcc lccc clang; do
    case "$name" in gcc) cc=$GCC;; lccc) cc=$LCCC;; clang) cc=$CLANG;; esac
    for f in "${lib_sources[@]}"; do
        echo "$name library $f"
        "$cc" "${flags[@]}" -c "$src/lib/$f" -o "$out/$name-lib/${f%.c}.o"
    done
done
for name in gcc lccc; do
    case "$name" in gcc) cc=$GCC;; lccc) cc=$LCCC;; esac
    for f in "${test_sources[@]}"; do
        echo "$name tests $f"
        "$cc" "${flags[@]}" -c "$src/tests/$f" -o "$out/$name-tests/${f%.c}.o"
    done
done
for name in gcc lccc clang; do
    "$GCC" "$out"/gcc-tests/*.o "$out/$name-lib"/*.o -lm -o "$out/runtests-$name"
    (cd "$out" && ./"runtests-$name" > "runtests-$name.log" 2>&1)
    grep -F '100%: Checks: 5196, Failed: 0' "$out/runtests-$name.log"
done
"$GCC" "$out"/lccc-tests/*.o "$out"/lccc-lib/*.o -lm -o "$out/runtests-all-lccc"
(cd "$out" && ./runtests-all-lccc > runtests-all-lccc.log 2>&1)
grep -F '100%: Checks: 5196, Failed: 0' "$out/runtests-all-lccc.log"

python3 - "$out/corpus-aligned.xml" <<'PY'
import sys
from pathlib import Path
parts = ['<?xml version="1.0" encoding="UTF-8"?><feed xmlns="urn:example:feed" xmlns:x="urn:example:meta">\n']
for i in range(10000):
    parts.append(f'<entry id="{i}" x:kind="sample"><title>item {i} &amp; naïve</title><value>{i*17%997}</value><x:link href="https://example.invalid/{i%71}"/></entry>\n')
parts.append('</feed>\n')
raw = ''.join(parts).encode('utf-8')
# The upstream benchmark advances by a whole buffer after its last parse.
# Padding with legal trailing XML whitespace keeps that pointer at one-past-end.
raw += b' ' * ((-len(raw)) % 8192)
Path(sys.argv[1]).write_bytes(raw)
PY
(cd "$out" && printf '%s  corpus-aligned.xml\n' ad11dae6d1a4661377dc3531105fe6b177876186197f1721b3c7fb38eb3e944e | sha256sum -c)
"$GCC" "${flags[@]}" -c "$src/tests/benchmark/benchmark.c" -o "$out/benchmark.o"
"$GCC" "${flags[@]}" -c "$here/event_check.c" -o "$out/event_check.o"
for name in gcc lccc clang; do
    "$GCC" "$out/benchmark.o" "$out/$name-lib"/*.o -lm -o "$out/benchmark-$name"
    "$GCC" "$out/event_check.o" "$out/$name-lib"/*.o -lm -o "$out/event-$name"
    "$out/event-$name" "$out/corpus-aligned.xml" > "$out/event-$name.out"
    grep -Fx '40001 40001 217784 7f21649baacdab7c' "$out/event-$name.out"
done
cmp "$out/event-gcc.out" "$out/event-lccc.out"
cmp "$out/event-gcc.out" "$out/event-clang.out"
python3 "$here/paired.py" "$out" --rounds "${ROUNDS:-40}"

if [[ -n ${VALGRIND:-} ]]; then
    "$VALGRIND" --version
    for name in gcc lccc clang; do
        "$VALGRIND" --tool=callgrind --cache-sim=yes --branch-sim=yes \
            --I1=32768,8,64 --D1=32768,8,64 --LL=33554432,16,64 \
            --callgrind-out-file="$out/callgrind-$name.out" \
            "$out/benchmark-$name" -n "$out/corpus-aligned.xml" 8192 2 \
            > "$out/callgrind-$name.stdout" 2> "$out/callgrind-$name.stderr"
        grep -E '^events:|^summary:' "$out/callgrind-$name.out"
    done
else
    echo 'VALGRIND not set; Callgrind was not run.'
fi
