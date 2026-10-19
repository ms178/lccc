#!/usr/bin/env bash
# Build glibc 2.44 with LCCC and the companion lccc-ld.
#
# This reproduces the toolchain-stable/glibc recipe in a non-destructive
# staging tree.  The recipe is 2.44 plus 25 commits, not the 2.44 release
# tarball: source commit, archive bytes, archpkgbuilds revision, and patch bytes
# are therefore all pinned and checked before any build or reuse.  The patch is
# applied with --fuzz=0.  The compiler is the project's fastbuild LCCC (target
# code at -O2, compiler itself at Rust -O1),
# and installs only below WORK.  It never writes /usr or replaces the host
# libc.  Set GLIBC_RUN_SMOKE=1 to additionally run a program with the staged
# loader; the default is structural/build validation because a staged glibc
# must be paired with a complete sysroot before runtime claims are meaningful.
#
# The current LCCC linker has no IFUNC/SFrame emission, LCCC cannot assemble
# glibc's AVX-512 libmvec sources, and semantic-interposition policy is not an
# implemented driver option. The capability-gated defaults therefore disable
# multi-arch, mathvec, and SFrame while keeping a scalar libc/libm build. Remove
# those gates only after the relevant LCCC features pass their own differential
# tests.
#
# Usage:
#   tests/workloads/glibc-2.44/build_lccc.sh
#   GLIBC_WORK_DIR=/large/disk/path GLIBC_RUN_SMOKE=1 build_lccc.sh
set -euo pipefail

ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/../../.." && pwd)
LCCC=${LCCC:-$ROOT/target/fastbuild/lccc}
LCCC_LD=${LCCC_LD:-$(dirname "$LCCC")/lccc-ld}
WORK=${GLIBC_WORK_DIR:-/home/user/workloads/glibc-2.44}
CACHE=${GLIBC_CACHE:-/home/user/source-cache}
GLIBC_COMMIT=afd131806b25715a7617ab757db43f0bb610acbf
TARBALL=${GLIBC_TARBALL:-$CACHE/glibc-$GLIBC_COMMIT.tar.gz}
TARBALL_URL=https://github.com/bminor/glibc/archive/$GLIBC_COMMIT.tar.gz
TARBALL_SHA256=764ea4a5f2aebf9d9b61f07a50a1f54854055a913dd66a0f05e1d0a9b7f08bb2
EXPECTED_ARCHPKGBUILDS_REV=4587e7caa08ada404dd603be8ae066ae8745220e
PATCH=${MS178_GLIBC_PATCH:-/home/user/archpkgbuilds/toolchain-stable/glibc/ms178-glibc.patch}
PATCH_SHA256=52c9953f8e3ec710d73308464bd98bbe752895d83c6b17201235edcb1c974aeb
BINUTILS=${GLIBC_BINUTILS_PREFIX:-/home/user/tools/binutils-2.47/bin}
JOBS=${JOBS:-2}
# C and C++ host tools are a pair: glibc's support helper uses the C++
# compiler's private headers, while the smoke setup queries the C compiler.
# Requiring both overrides prevents an accidental gcc/clang split.
if [[ ${HOST_CC+x} != "${HOST_CXX+x}" ]]; then
    echo 'error: set HOST_CC and HOST_CXX together' >&2
    exit 2
fi
HOST_CC=${HOST_CC:-$(command -v gcc || true)}
HOST_CXX=${HOST_CXX:-$(command -v g++ || true)}
# The resulting glibc is loaded at process startup, so Initial-Exec TLS is
# valid and avoids Local-Dynamic __tls_get_addr sequences. It is not a safe
# default for arbitrary DSOs that may be dlopen'd after threads exist.
CFLAGS=${GLIBC_CFLAGS:--O2 -march=x86-64-v3 -mtune=native -ftls-model=initial-exec -fomit-frame-pointer -fcf-protection=none -mharden-sls=none -fno-stack-protector}

mkdir -p "$WORK" "$CACHE"
"$ROOT/scripts/ensure_swap.sh"
[[ -x "$LCCC" ]] || { echo "error: LCCC is not executable: $LCCC" >&2; exit 2; }
[[ -x "$LCCC_LD" ]] || { echo "error: lccc-ld is not executable: $LCCC_LD" >&2; exit 2; }
[[ -x "$HOST_CC" ]] || { echo "error: HOST_CC is not executable: $HOST_CC" >&2; exit 2; }
[[ -x "$HOST_CXX" ]] || { echo "error: HOST_CXX is not executable: $HOST_CXX" >&2; exit 2; }
host_cc_target=$($HOST_CC -dumpmachine)
host_cxx_target=$($HOST_CXX -dumpmachine)
[[ $host_cc_target == "$host_cxx_target" ]] || {
    echo "error: incoherent host compiler targets: CC=$host_cc_target CXX=$host_cxx_target" >&2
    exit 2
}
[[ -f "$PATCH" ]] || { echo "error: ms178 patch not found: $PATCH" >&2; exit 2; }
patch_sha=$(sha256sum "$PATCH" | awk '{print $1}')
[[ "$patch_sha" == "$PATCH_SHA256" ]] || {
    echo "error: ms178 patch checksum mismatch: $patch_sha (expected $PATCH_SHA256)" >&2
    exit 1
}
# Capture provenance from the checkout that actually supplied the patch.  The
# checksum remains the content authority; when this is the pinned
# archpkgbuilds checkout, also reject a stale/wrong Git revision.
ARCHPKGBUILDS_REV=not-a-git-checkout
ARCHPKGBUILDS_DIR=
if ARCHPKGBUILDS_DIR=$(git -C "$(dirname "$PATCH")" rev-parse --show-toplevel 2>/dev/null); then
    ARCHPKGBUILDS_REV=$(git -C "$ARCHPKGBUILDS_DIR" rev-parse HEAD)
    [[ $ARCHPKGBUILDS_REV == "$EXPECTED_ARCHPKGBUILDS_REV" ]] || {
        echo "error: archpkgbuilds revision $ARCHPKGBUILDS_REV (expected $EXPECTED_ARCHPKGBUILDS_REV)" >&2
        exit 1
    }
fi
# A path containing "2.47" proves nothing.  Every binutils program the build
# invokes is independently checked with the shared exact-token policy.
# ROOT is computed from this script's location.
# shellcheck disable=SC1091
source "$ROOT/tools/linker/oracle_versions.sh"
for tool in as ld ar ranlib objdump readelf; do
    [[ -x "$BINUTILS/$tool" ]] || {
        echo "error: binutils tool missing: $BINUTILS/$tool" >&2
        exit 2
    }
    oracle_require_version "$BINUTILS/$tool" 2.47 || exit 2
done

if [[ ! -f "$TARBALL" ]]; then
    tmp="$TARBALL.tmp.$$"
    trap 'rm -f "$tmp"' EXIT
    curl -fL --retry 3 -o "$tmp" "$TARBALL_URL"
    mv -f "$tmp" "$TARBALL"
    trap - EXIT
fi
archive_sha=$(sha256sum "$TARBALL" | awk '{print $1}')
[[ "$archive_sha" == "$TARBALL_SHA256" ]] || {
    echo "error: glibc archive checksum mismatch: $archive_sha" >&2
    exit 1
}

SRC="$WORK/glibc-$GLIBC_COMMIT"
BUILD="$WORK/build-lccc"
STAGE="$WORK/stage-lccc"

# Reuse is an explicit recovery mode: it lets a harness restart after an
# interrupted/failed post-build gate without paying another multi-minute glibc
# compile.  It is safe only when every build input agrees with the receipt from
# the completed build; path or version-string equality is insufficient.
if [[ ${GLIBC_REUSE_BUILD:-0} == 1 ]]; then
    python3 - "$WORK/manifest.json" "$archive_sha" "$patch_sha" "$LCCC" "$LCCC_LD" "$BINUTILS" "$CFLAGS" "$GLIBC_COMMIT" "$ARCHPKGBUILDS_REV" "$HOST_CC" "$HOST_CXX" <<'PY'
import hashlib, json, pathlib, sys
(manifest, archive, patch, lccc, ld, binutils, cflags, commit,
 archpkgbuilds_rev, host_cc, host_cxx) = sys.argv[1:]
def digest(path):
    h = hashlib.sha256()
    with open(path, 'rb') as f:
        for block in iter(lambda: f.read(1024 * 1024), b''): h.update(block)
    return h.hexdigest()
p = pathlib.Path(manifest)
if not p.is_file(): raise SystemExit(f'error: reuse manifest missing: {p}')
got = json.loads(p.read_text())
expected = {
    'schema': 2, 'source_commit': commit,
    'archpkgbuilds_revision': archpkgbuilds_rev, 'archive_sha256': archive,
    'ms178_patch_sha256': patch, 'cflags': cflags,
    'lccc': str(pathlib.Path(lccc).resolve()), 'lccc_sha256': digest(lccc),
    'lccc_ld': str(pathlib.Path(ld).resolve()), 'lccc_ld_sha256': digest(ld),
    'binutils_247': str(pathlib.Path(binutils).resolve()),
    'host_cc': str(pathlib.Path(host_cc).resolve()), 'host_cc_sha256': digest(host_cc),
    'host_cxx': str(pathlib.Path(host_cxx).resolve()), 'host_cxx_sha256': digest(host_cxx),
    'binutils_sha256': {name: digest(str(pathlib.Path(binutils) / name))
                        for name in ('as', 'ld', 'ar', 'ranlib', 'objdump', 'readelf')},
}
diffs = [f'{key}: receipt={got.get(key)!r}, current={value!r}'
         for key, value in expected.items() if got.get(key) != value]
if diffs:
    raise SystemExit('error: refusing stale GLIBC_REUSE_BUILD:\n  ' + '\n  '.join(diffs))
PY
fi
if [[ ${GLIBC_REUSE_BUILD:-0} != 1 ]]; then
    # rm -rf is intentionally confined to three known children of WORK.  This
    # catches empty/root/home overrides and symlink/.. escapes before deleting.
    work_real=$(realpath -m -- "$WORK")
    [[ $work_real != / && $work_real != "$HOME" ]] || {
        echo "error: refusing destructive GLIBC_WORK_DIR: $work_real" >&2
        exit 2
    }
    safe_remove_tree() {
        local candidate_real
        candidate_real=$(realpath -m -- "$1")
        case "$candidate_real" in
            "$work_real"/glibc-*|"$work_real"/build-lccc|"$work_real"/stage-lccc)
                rm -rf --one-file-system -- "$candidate_real" ;;
            *) echo "error: refusing rm outside guarded work children: $candidate_real" >&2; exit 2 ;;
        esac
    }
    safe_remove_tree "$SRC"
    safe_remove_tree "$BUILD"
    safe_remove_tree "$STAGE"
    tar -xf "$TARBALL" -C "$WORK"
    patch --dry-run -Np1 --fuzz=0 -i "$PATCH" -d "$SRC" >/dev/null
    patch -Np1 --fuzz=0 -i "$PATCH" -d "$SRC" >/dev/null
    mkdir -p "$BUILD"

    # CXX needs GCC's internal and multiarch headers because glibc's support
    # helper is compiled with -nostdinc.  Query the selected host compiler;
    # hardcoding Debian's GCC 14 paths breaks Sid and non-Debian hosts.
    gcc_include=$($HOST_CXX -print-file-name=include)
    multiarch=$($HOST_CXX -print-multiarch)
    [[ -d "$gcc_include" ]] || { echo "error: GCC include dir missing: $gcc_include" >&2; exit 2; }
    CXX="$HOST_CXX -isystem $gcc_include"
    [[ -z "$multiarch" || ! -d "/usr/include/$multiarch" ]] || CXX+=" -isystem /usr/include/$multiarch"
    BINUTILS_PATH="$BINUTILS:$(dirname "$LCCC"):/usr/bin"
    BUILD_CFLAGS=$CFLAGS
    {
        printf 'source_commit=%s\narchive_sha256=%s\npatch_sha256=%s\n' \
            "$GLIBC_COMMIT" "$archive_sha" "$patch_sha"
        printf 'archpkgbuilds_revision=%s\nhost_cc=%s\nhost_cxx=%s\nhost_target=%s\n' \
            "$ARCHPKGBUILDS_REV" "$HOST_CC" "$HOST_CXX" "$host_cc_target"
        "$HOST_CC" --version | head -n1
        "$HOST_CXX" --version | head -n1
    } | tee "$BUILD/inputs.log"
    report_build_failure() {
        local status=$?
        echo "error: glibc build failed (status $status); log tails:" >&2
        tail -n 40 "$BUILD"/{configure,build,install}.log 2>/dev/null >&2 || true
        exit "$status"
    }
    trap report_build_failure ERR
    (
        cd "$BUILD"
        PATH="$BINUTILS_PATH" CC="$LCCC" CXX="$CXX" LD="$LCCC_LD" \
          AS="$BINUTILS/as" AR="$BINUTILS/ar" RANLIB="$BINUTILS/ranlib" \
          OBJDUMP="$BINUTILS/objdump" READELF="$BINUTILS/readelf" \
          CPPFLAGS='-D_FORTIFY_SOURCE=0' CFLAGS="$BUILD_CFLAGS" CXXFLAGS="$BUILD_CFLAGS" \
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
    trap - ERR
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
    gcc_include=$($HOST_CC -print-file-name=include)
    multiarch=$($HOST_CC -print-multiarch)
    smoke_includes=(-isystem "$INSTALL/include" -isystem "$gcc_include" -isystem /usr/include)
    [[ -z "$multiarch" || ! -d "/usr/include/$multiarch" ]] || smoke_includes+=(-isystem "/usr/include/$multiarch")
    "$LCCC" -O2 -nostdinc "${smoke_includes[@]}" \
      "$WORK/smoke.c" -L"$INSTALL/lib" \
      -Wl,-rpath,"$INSTALL/lib" \
      -Wl,--dynamic-linker="$INSTALL/lib/ld-1-x86-64.so.2" -lc -lm \
      -o "$WORK/smoke-lccc"
    LD_LIBRARY_PATH="$INSTALL/lib" "$WORK/smoke-lccc"
fi

python3 - "$WORK/manifest.json" "$archive_sha" "$patch_sha" "$LCCC" "$LCCC_LD" "$BINUTILS" "$INSTALL" "$CFLAGS" "$GLIBC_COMMIT" "$ARCHPKGBUILDS_REV" "$HOST_CC" "$HOST_CXX" <<'PY'
import hashlib, json, pathlib, subprocess, sys
(out, archive, patch, lccc, ld, binutils, install, cflags, commit,
 archpkgbuilds_rev, host_cc, host_cxx) = sys.argv[1:]
def digest(path):
    h = hashlib.sha256()
    with open(path, 'rb') as f:
        for block in iter(lambda: f.read(1024 * 1024), b''): h.update(block)
    return h.hexdigest()
def v(cmd):
    try: return subprocess.run([cmd, '--version'], capture_output=True, text=True).stdout.splitlines()[0]
    except OSError as e: return f'{type(e).__name__}: {e}'
p = {
  'schema': 2, 'workload': 'glibc', 'version': '2.44+r25+gafd131806b',
  'source_commit': commit, 'archpkgbuilds_revision': archpkgbuilds_rev,
  'archive_sha256': archive, 'ms178_patch_sha256': patch, 'cflags': cflags,
  'lccc': str(pathlib.Path(lccc).resolve()), 'lccc_sha256': digest(lccc),
  'lccc_version': v(lccc),
  'lccc_ld': str(pathlib.Path(ld).resolve()), 'lccc_ld_sha256': digest(ld),
  'lccc_ld_version': v(ld),
  'binutils_247': str(pathlib.Path(binutils).resolve()),
  'host_cc': str(pathlib.Path(host_cc).resolve()), 'host_cc_sha256': digest(host_cc),
  'host_cc_version': v(host_cc),
  'host_cxx': str(pathlib.Path(host_cxx).resolve()), 'host_cxx_sha256': digest(host_cxx),
  'host_cxx_version': v(host_cxx),
  'binutils_sha256': {name: digest(str(pathlib.Path(binutils) / name))
                      for name in ('as', 'ld', 'ar', 'ranlib', 'objdump', 'readelf')},
  'capability_gates': ['--disable-multi-arch', '--disable-mathvec', '--disable-sframe'],
  'kernel_compat_baseline': '3.2 (host headers do not advertise the recipe 6.18 baseline)',
  'stage': install, 'result': 'PASS',
}
pathlib.Path(out).write_text(json.dumps(p, indent=2) + '\n')
PY
printf 'glibc 2.44 LCCC build/install: PASS\n'
printf '  archive=%s\n  patch=%s\n  stage=%s\n' "$archive_sha" "$patch_sha" "$INSTALL"
