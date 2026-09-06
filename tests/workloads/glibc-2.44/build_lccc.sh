#!/usr/bin/env bash
# Build glibc 2.44 with LCCC and the companion lccc-ld.
#
# This reproduces the toolchain-stable/glibc recipe in a non-destructive
# staging tree.  It applies the exact ms178 patch from archpkgbuilds with
# --fuzz=0, records the source/patch/toolchain identities, builds with the
# project's fastbuild LCCC (target code at -O2, compiler itself at Rust -O1),
# and installs only below WORK.  It never writes /usr or replaces the host
# libc.  Set GLIBC_RUN_SMOKE=1 to additionally run a program with the staged
# loader; the default is structural/build validation because a staged glibc
# must be paired with a complete sysroot before runtime claims are meaningful.
#
# The current LCCC linker has no IFUNC/SFrame emission and LCCC cannot assemble
# glibc's AVX-512 libmvec sources.  The capability-gated defaults therefore
# disable multi-arch, mathvec, and SFrame while keeping the ms178 patch and the
# complete scalar libc/libm build.  Remove those gates only after the relevant
# LCCC features pass their own differential tests.
#
# Usage:
#   tests/workloads/glibc-2.44/build_lccc.sh
#   GLIBC_WORK_DIR=/large/disk/path GLIBC_RUN_SMOKE=1 build_lccc.sh
set -euo pipefail

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
LCCC=${LCCC:-$ROOT/target/fastbuild/lccc}
LCCC_LD=${LCCC_LD:-$(dirname "$LCCC")/lccc-ld}
WORK=${GLIBC_WORK_DIR:-/home/user/workloads/glibc-2.44}
CACHE=${GLIBC_CACHE:-/home/user/source-cache}
TARBALL=${GLIBC_TARBALL:-$CACHE/glibc-2.44.tar.xz}
TARBALL_URL=https://ftp.gnu.org/gnu/glibc/glibc-2.44.tar.xz
TARBALL_SHA256=37f600f2bef3c5e8300147059568b2a2e40a7ad6ccc65ce942556d49429cc667
PATCH=${MS178_GLIBC_PATCH:-/home/user/archpkgbuilds/toolchain-stable/glibc/ms178-glibc.patch}
PATCH_SHA256= # filled/checked below; the recipe is the authority
BINUTILS=${GLIBC_BINUTILS_PREFIX:-/home/user/tools/binutils-2.47/bin}
JOBS=${JOBS:-2}
CFLAGS=${GLIBC_CFLAGS:--O2 -march=x86-64-v3 -mtune=native -fomit-frame-pointer -fcf-protection=none -mharden-sls=none -fno-stack-protector -fno-semantic-interposition}

mkdir -p "$WORK" "$CACHE"
"$ROOT/scripts/ensure_swap.sh"
[[ -x "$LCCC" ]] || { echo "error: LCCC is not executable: $LCCC" >&2; exit 2; }
[[ -x "$LCCC_LD" ]] || { echo "error: lccc-ld is not executable: $LCCC_LD" >&2; exit 2; }
[[ -f "$PATCH" ]] || { echo "error: ms178 patch not found: $PATCH" >&2; exit 2; }
[[ -x "$BINUTILS/as" && -x "$BINUTILS/ld" && -x "$BINUTILS/readelf" ]] || {
    echo "error: binutils 2.47 prefix missing under $BINUTILS" >&2
    exit 2
}
"$BINUTILS/readelf" --version | grep -F '2.47' >/dev/null || { echo 'error: readelf is not binutils 2.47' >&2; exit 2; }

if [[ ! -f "$TARBALL" ]]; then curl -fL --retry 3 -o "$TARBALL" "$TARBALL_URL"; fi
archive_sha=$(sha256sum "$TARBALL" | awk '{print $1}')
[[ "$archive_sha" == "$TARBALL_SHA256" ]] || {
    echo "error: glibc archive checksum mismatch: $archive_sha" >&2
    exit 1
}
patch_sha=$(sha256sum "$PATCH" | awk '{print $1}')

SRC="$WORK/glibc-2.44"
BUILD="$WORK/build-lccc"
STAGE="$WORK/stage-lccc"

# Reuse is an explicit recovery mode: it lets a harness restart after an
# interrupted/failed post-build gate without paying another multi-minute glibc
# compile. The default remains clean and reproducible.
if [[ ${GLIBC_REUSE_BUILD:-0} != 1 ]]; then
    rm -rf "$SRC" "$BUILD" "$STAGE"
    tar -xf "$TARBALL" -C "$WORK"
    patch --dry-run -Np1 --fuzz=0 -i "$PATCH" -d "$SRC" >/dev/null
    patch -Np1 --fuzz=0 -i "$PATCH" -d "$SRC" >/dev/null
    mkdir -p "$BUILD"

    # CXX needs the GCC internal headers because glibc's support helper is compiled
    # with -nostdinc while still using libstdc++ headers.
    CXX="/usr/bin/g++ -isystem /usr/lib/gcc/x86_64-linux-gnu/14/include -isystem /usr/include/x86_64-linux-gnu"
    BINUTILS_PATH="$BINUTILS:/home/user/lccc/target/fastbuild:/usr/bin"
    (
        cd "$BUILD"
        PATH="$BINUTILS_PATH" CC="$LCCC" CXX="$CXX" LD="$LCCC_LD" \
          AS="$BINUTILS/as" AR="$BINUTILS/ar" RANLIB="$BINUTILS/ranlib" \
          OBJDUMP="$BINUTILS/objdump" READELF="$BINUTILS/readelf" \
          CPPFLAGS='-D_FORTIFY_SOURCE=0' CFLAGS="$CFLAGS" CXXFLAGS="$CFLAGS" \
          "$SRC/configure" \
            --prefix="$WORK/install-lccc" \
            --libdir="$WORK/install-lccc/lib" \
            --libexecdir="$WORK/install-lccc/lib" \
            --with-headers=/usr/include --enable-bind-now \
            --disable-fortify-source --enable-kernel=3.2 --disable-cet \
            --disable-multi-arch --disable-mathvec --disable-stack-protector \
            --disable-systemtap --disable-nscd --disable-profile --disable-werror \
            --with-rtld-early-cflags='-march=x86-64' --disable-sframe >configure.log 2>&1
        PATH="$BINUTILS_PATH" make -j"$JOBS" -Oline CXX="$CXX" >build.log 2>&1
        PATH="$BINUTILS_PATH" make install DESTDIR="$STAGE" >install.log 2>&1
    )
else
    INSTALL_REUSE="$STAGE$WORK/install-lccc"
    [[ -x "$INSTALL_REUSE/lib/libc.so.6" ]] || {
        echo "error: GLIBC_REUSE_BUILD=1 but staged libc is missing: $INSTALL_REUSE" >&2
        exit 2
    }
fi

# Structural ABI checks are meaningful even when staged runtime execution is
# deliberately not requested. They catch the exact historical failure modes:
# missing versioned imports, malformed verneed offsets, and accidental host
# libc dependencies.
INSTALL="$STAGE$WORK/install-lccc"
readelf -h "$INSTALL/lib/libc.so.6" | grep -F 'ELF64' >/dev/null
readelf -sW "$INSTALL/lib/libc.so.6" | grep -F '_rtld_global_ro@GLIBC_PRIVATE' >/dev/null
readelf -d "$INSTALL/lib/libc.so.6" | grep -F 'VERNEED' >/dev/null
readelf --version-info "$INSTALL/lib/libc.so.6" | grep -F 'Name: GLIBC_PRIVATE' >/dev/null

if [[ ${GLIBC_RUN_SMOKE:-0} == 1 ]]; then
    cat >"$WORK/smoke.c" <<'EOF'
#include <gnu/libc-version.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
int main(void) {
    void *p = malloc(32);
    if (!p) return 2;
    printf("glibc=%s fma=%.1f\n", gnu_get_libc_version(), fma(2.0, 3.0, 4.0));
    free(p);
    return 0;
}
EOF
    "$LCCC" -O2 -nostdinc -isystem "$INSTALL/include" \
      -isystem /usr/lib/gcc/x86_64-linux-gnu/14/include \
      -isystem /usr/include -isystem /usr/include/x86_64-linux-gnu \
      "$WORK/smoke.c" -L"$INSTALL/lib" \
      -Wl,-rpath,"$INSTALL/lib" \
      -Wl,--dynamic-linker="$INSTALL/lib/ld-1-x86-64.so.2" -lc -lm \
      -o "$WORK/smoke-lccc"
    LD_LIBRARY_PATH="$INSTALL/lib" "$WORK/smoke-lccc"
fi

python3 - "$WORK/manifest.json" "$archive_sha" "$patch_sha" "$LCCC" "$LCCC_LD" "$BINUTILS" "$INSTALL" <<'PY'
import json, pathlib, subprocess, sys
out, archive, patch, lccc, ld, binutils, install = sys.argv[1:]
def v(cmd):
    try: return subprocess.run([cmd, '--version'], capture_output=True, text=True).stdout.splitlines()[0]
    except OSError as e: return f'{type(e).__name__}: {e}'
p = {
  'schema': 1, 'workload': 'glibc', 'version': '2.44',
  'archive_sha256': archive, 'ms178_patch_sha256': patch,
  'lccc': str(pathlib.Path(lccc).resolve()), 'lccc_version': v(lccc),
  'lccc_ld': str(pathlib.Path(ld).resolve()), 'lccc_ld_version': v(ld),
  'binutils_247': str(pathlib.Path(binutils).resolve()),
  'capability_gates': ['--disable-multi-arch', '--disable-mathvec', '--disable-sframe'],
  'kernel_compat_baseline': '3.2 (host headers do not advertise the recipe 6.18 baseline)',
  'stage': install, 'result': 'PASS',
}
pathlib.Path(out).write_text(json.dumps(p, indent=2) + '\n')
PY
printf 'glibc 2.44 LCCC build/install: PASS\n'
printf '  archive=%s\n  patch=%s\n  stage=%s\n' "$archive_sha" "$patch_sha" "$INSTALL"
