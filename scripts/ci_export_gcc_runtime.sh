#!/usr/bin/env bash
# Expose the exact private GCC runtime through lccc's additive sysroot support.
# Never unpack over / or substitute an older distribution multilib compiler.
set -euo pipefail
compiler=${GCC_DRIVER:-gcc-16}
fail() { printf 'gcc-runtime: %s\n' "$*" >&2; exit 1; }
[[ $("$compiler" -dumpfullversion) = 16.2.0 ]] || fail "GCC 16.2.0 is required"
: "${RUNNER_TEMP:?RUNNER_TEMP is required}"
: "${GITHUB_ENV:?GITHUB_ENV is required}"
root="$RUNNER_TEMP/current-gcc-runtime"
[[ ! -e $root ]] || fail "refusing an existing runtime sysroot: $root"
crt64=$("$compiler" -print-file-name=crtbegin.o)
crt32=$("$compiler" -m32 -print-file-name=crtbegin.o)
lib64=$("$compiler" -print-libgcc-file-name)
lib32=$("$compiler" -m32 -print-libgcc-file-name)
shared64=$("$compiler" -print-file-name=libgcc_s.so)
shared32=$("$compiler" -m32 -print-file-name=libgcc_s.so)
soname64=$("$compiler" -print-file-name=libgcc_s.so.1)
soname32=$("$compiler" -m32 -print-file-name=libgcc_s.so.1)
for file in "$crt64" "$crt32" "$lib64" "$lib32" "$shared64" "$shared32" "$soname64" "$soname32"; do
    [[ $file = /* && -f $file ]] || fail "missing exact GCC runtime file: $file"
done
crt64=$(readlink -f "$crt64")
crt32=$(readlink -f "$crt32")
lib64=$(readlink -f "$lib64")
lib32=$(readlink -f "$lib32")
shared64=$(readlink -f "$shared64")
shared32=$(readlink -f "$shared32")
soname64=$(readlink -f "$soname64")
soname32=$(readlink -f "$soname32")
directory=$(dirname "$crt64")
[[ $crt32 = "$directory/32/crtbegin.o" && $lib64 = "$directory/libgcc.a" && $lib32 = "$directory/32/libgcc.a" ]] || \
    fail "expected one coherent GCC installation with native and 32/ multilib runtimes"
installation=$(cd "$directory/../../../.." && pwd -P)
[[ $shared64 = "$installation/lib64/libgcc_s.so" && $soname64 = "$installation/lib64/libgcc_s.so.1" && \
   $shared32 = "$installation/lib32/libgcc_s.so" && $soname32 = "$installation/lib32/libgcc_s.so.1" ]] || \
    fail "shared unwind runtimes must come from the same exact GCC installation"
# Both targets already understand this FHS layout; the directory is a symlink
# to authenticated compiler bytes, not a copied or modified runtime.
mkdir -p "$root/usr/lib/gcc/x86_64-linux-gnu"
ln -s "$directory" "$root/usr/lib/gcc/x86_64-linux-gnu/16.2.0"
ln -s "$installation/lib64" "$root/usr/lib/x86_64-linux-gnu"
ln -s "$installation/lib32" "$root/usr/lib/i386-linux-gnu"
python3 - "$root" "$compiler" "$crt64" "$crt32" "$lib64" "$lib32" \
    "$shared64" "$shared32" "$soname64" "$soname32" <<'PY'
import hashlib
import json
from pathlib import Path
import sys
root, compiler, *files = sys.argv[1:]
receipt = {'gcc_version': '16.2.0', 'driver': compiler, 'sysroot': root,
           'files': [{'path': name, 'sha256': hashlib.sha256(Path(name).read_bytes()).hexdigest()}
                     for name in files]}
Path(root, 'runtime.json').write_text(json.dumps(receipt, indent=2) + '\n')
PY
printf 'LCCC_SYSROOT=%s\n' "$root" >> "$GITHUB_ENV"
# Dynamic i686 links use the shared unwinder, never a private static copy.
# Both loader classes skip the incompatible architecture's directory.
printf 'LD_LIBRARY_PATH=%s/lib64:%s/lib32%s\n' "$installation" "$installation" \
    "${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" >> "$GITHUB_ENV"
printf '%s\n' "$root"
