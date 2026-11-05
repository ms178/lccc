#!/usr/bin/env bash
# Publish an immutable, GitHub-only toolchain cache tag for network-constrained
# agents.  Everything in the tag is reachable by `git fetch origin tag ...`;
# no release-assets CDN, Debian mirror, kernel.org or Arm CDN is needed by the
# consumer.  Run this on a host with full egress (normally a workflow_dispatch
# job); all bytes are verified before they enter the tag.
set -Eeuo pipefail

TAG=${TAG:-harness-cache-8}
REMOTE=${REMOTE:-origin}
WORK=${WORK:-${RUNNER_TEMP:-/tmp}/agent-toolchain-cache}
REPO=${GITHUB_REPOSITORY:-ms178/lccc}
PART_SIZE=$((90 * 1024 * 1024))
PUSH=${PUSH:-1}

while [ $# -gt 0 ]; do
    case $1 in
        --tag) TAG=$2; shift 2 ;;
        --work) WORK=$2; shift 2 ;;
        --remote) REMOTE=$2; shift 2 ;;
        --no-push) PUSH=0; shift ;;
        -h|--help) sed -n '2,30p' "$0"; exit 0 ;;
        *) echo "publish_agent_toolchain_cache: unknown argument: $1" >&2; exit 2 ;;
    esac
done

log() { printf '[agent-cache] %s\n' "$*" >&2; }
die() { printf '[agent-cache] FATAL: %s\n' "$*" >&2; exit 1; }
need() { for t in "$@"; do command -v "$t" >/dev/null 2>&1 || die "missing tool: $t"; done; }

# A raw job log is not always reachable from the check-run API, which is what a
# reviewer of a red publisher run actually sees. Report the failing command and
# line as an annotation so the next failure is diagnosable without the log.
on_error() {
    local rc=$? line=${BASH_LINENO[0]:-?} cmd=${BASH_COMMAND:-?}
    printf '::error title=agent-cache publisher::line %s: %s (exit %s)\n' \
        "$line" "$cmd" "$rc" >&2
    exit "$rc"
}
trap on_error ERR

need git curl tar xz zstd sha256sum split dpkg-deb python3 gh gcc cc make docker
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"

WORK=$(realpath -m "$WORK")
rm -rf "$WORK"
mkdir -p "$WORK"/{src,files,stage,tree,extract,test}
SRC="$WORK/src"; FILES="$WORK/files"; STAGE="$WORK/stage"; TREE="$WORK/tree"; EXTRACT="$WORK/extract"; TEST="$WORK/test"

sha256() { sha256sum "$1" | awk '{print $1}'; }
verify() { local f=$1 want=$2 got; got=$(sha256 "$f"); [ "$got" = "$want" ] || die "SHA-256 mismatch for $(basename "$f"): got $got want $want"; }
fetch() { curl -fL --retry 5 --retry-delay 2 --retry-all-errors -o "$1" "$2"; }

# Existing immutable transport tags are the cheapest source of GCC, the
# bookworm runtime debs, the binutils source tarball, Arm ISA XML and the MSRV
# Rust components.  Their manifests are the trust root.
for t in harness-cache-1 harness-cache-2 harness-cache-3 harness-cache-4; do
    log "fetching $t"
    git fetch --no-tags "$REMOTE" "refs/tags/$t:refs/tags/$t" >/dev/null 2>&1 || git fetch --no-tags "$REMOTE" "refs/tags/$t:refs/tags/$t"
done

blob() { git cat-file blob "$1:$2" > "$3"; }

log "materialising GCC 16.2.0 + bookworm runtime + binutils source from harness-cache-1"
blob harness-cache-1 gcc-16-bookworm-x86_64.tar.xz.part00 "$FILES/gcc-16.2.0-bookworm-x86_64.tar.xz"
blob harness-cache-1 debs-bookworm.tar.xz.part00 "$FILES/debs-bookworm.tar.xz"
blob harness-cache-1 binutils-2.47.tar.xz.part00 "$FILES/binutils-2.47-source.tar.xz"
verify "$FILES/gcc-16.2.0-bookworm-x86_64.tar.xz" 4fa6af4f6b29991e31631af18d83964a69caa9daf3f78546d4dc30f39880cd8a
verify "$FILES/debs-bookworm.tar.xz" 24a4be08d5c6a28aed78f424ebdc9596f85cc2e8ebd0ba230f99a2bf9cdfb20d
verify "$FILES/binutils-2.47-source.tar.xz" 154ab23b60070e8f27013c22977f1129425d67d1e8acd6e13010e617811e4cff

log "materialising Arm A64 ISA XML 2026-09 from harness-cache-2"
blob harness-cache-2 ISA_A64_xml_A_profile-2026-09_md.tar.gz "$FILES/ISA_A64_xml_A_profile-2026-09_md.tar.gz"
verify "$FILES/ISA_A64_xml_A_profile-2026-09_md.tar.gz" 62917cba0da668deb6c28005231f4b94d1044cd52dbe4a4729fcc26d1642d827

log "downloading the locked Rust 1.99.1 release asset"
RUST_SHA=$(python3 -c 'import json;print(json.load(open("engineering/toolchains/cache-lock.json"))["tools"]["rust"]["sha256"])')
gh release download harness-cache-4 --repo "$REPO" --pattern 'rust-1.99.1-*.tar.xz' --dir "$SRC" --clobber
rust_asset=$(find "$SRC" -maxdepth 1 -name 'rust-1.99.1-*.tar.xz' -print -quit)
[ -n "$rust_asset" ] || die "locked Rust release asset not downloaded"
verify "$rust_asset" "$RUST_SHA"
cp "$rust_asset" "$FILES/rust-1.99.1-x86_64-unknown-linux-gnu.tar.xz"

log "downloading mold 3.0.0 from its pinned upstream release"
MOLD_SHA=$(sed -n 's/^SHA256=\([0-9a-f]\{64\}\)$/\1/p' scripts/ci_install_mold_oracle.sh)
fetch "$SRC/mold-3.0.0-x86_64-linux.tar.gz" https://github.com/rui314/mold/releases/download/v3.0.0/mold-3.0.0-x86_64-linux.tar.gz
verify "$SRC/mold-3.0.0-x86_64-linux.tar.gz" "$MOLD_SHA"
cp "$SRC/mold-3.0.0-x86_64-linux.tar.gz" "$FILES/mold-3.0.0-x86_64-linux.tar.gz"

# -- Clang: the LIVE pin, staged from the digest-verified official asset -------
# Deriving the version from the single enforcing file (rather than repeating a
# release here) is what stops a published tag from silently carrying a
# superseded oracle. The installer verifies the SHA-256 GitHub publishes for
# that exact asset, so the bytes entering the tag are the bytes CI installs.
CLANG_VERSION=$(sed -n 's/^VERSION=\([0-9.]*\)$/\1/p' scripts/ci_install_clang_oracle.sh)
[ -n "$CLANG_VERSION" ] || die "cannot read the Clang pin from scripts/ci_install_clang_oracle.sh"
CLANG_ROOT="clang-$CLANG_VERSION"
log "installing the pinned Clang $CLANG_VERSION oracle from the official LLVM asset"
rm -rf "$WORK/clang-prefix" "$STAGE/$CLANG_ROOT"
bash scripts/ci_install_clang_oracle.sh --prefix "$WORK/clang-prefix"
[ -x "$WORK/clang-prefix/bin/clang" ] || die "Clang installer produced no driver"
staged_clang=$("$WORK/clang-prefix/bin/clang" -dumpversion)
[ "$staged_clang" = "$CLANG_VERSION" ] || \
    die "staged Clang reports $staged_clang, expected $CLANG_VERSION"
mkdir -p "$STAGE/$CLANG_ROOT"
cp -a "$WORK/clang-prefix/." "$STAGE/$CLANG_ROOT/"
printf 'Clang %s - official LLVM asset, verified by scripts/ci_install_clang_oracle.sh\n' \
    "$CLANG_VERSION" > "$STAGE/$CLANG_ROOT/PIN"
tar --sort=name --mtime=@0 --owner=0 --group=0 --numeric-owner -I 'xz -T2 -6' \
    -cf "$FILES/clang-$CLANG_VERSION-linux-x64.tar.xz" -C "$STAGE" "$CLANG_ROOT"

# -- GDB: pinned source build with -march=haswell ----------------------------
# The DWARF gates fail closed on a debugger that is absent or unpinned, so the
# cache must carry an exact GDB 18.1 built for the ISA contract LCCC ships.
# Reuse the very installer that enforces the pin (digest, march, ISA census,
# line-table probe) inside the same bookworm container as binutils: one build
# recipe, broad glibc compatibility, no duplicated configure flags.
GDB_VERSION=$(sed -n 's/^GDB_VERSION=\([0-9.]*\)$/\1/p' scripts/ci_install_gdb_oracle.sh)
[ -n "$GDB_VERSION" ] || die "cannot read the GDB pin from scripts/ci_install_gdb_oracle.sh"
GDB_ROOT="gdb-$GDB_VERSION"
log "building pinned GDB $GDB_VERSION (-march=haswell) in a bookworm container"
cp scripts/ci_install_gdb_oracle.sh "$WORK/gdb-installer.sh"
cat > "$WORK/build-gdb.sh" <<'EOS'
#!/bin/bash
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y --no-install-recommends build-essential curl ca-certificates \
    xz-utils make gcc g++ python3 binutils libgmp-dev libmpfr-dev \
    libreadline-dev zlib1g-dev libexpat1-dev >/dev/null
bash /work/gdb-installer.sh --prefix /work/gdb-prefix
EOS
chmod +x "$WORK/build-gdb.sh"
docker run --rm -v "$WORK:/work" -w /work debian:bookworm bash /work/build-gdb.sh
sudo chown -R "$(id -u):$(id -g)" "$WORK/gdb-prefix"
[ -x "$WORK/gdb-prefix/bin/gdb" ] || die "GDB installer produced no debugger"
gdb_banner=$("$WORK/gdb-prefix/bin/gdb" --version | head -n 1)
[ "$gdb_banner" = "GNU gdb (GDB) $GDB_VERSION" ] || \
    die "staged GDB reports '$gdb_banner', expected 'GNU gdb (GDB) $GDB_VERSION'"
grep -qx 'march=haswell' "$WORK/gdb-prefix/.lccc-build" || \
    die "staged GDB was not built with -march=haswell"
rm -rf "$STAGE/$GDB_ROOT"; mkdir -p "$STAGE/$GDB_ROOT"
cp -a "$WORK/gdb-prefix/." "$STAGE/$GDB_ROOT/"
tar --sort=name --mtime=@0 --owner=0 --group=0 --numeric-owner -I 'xz -T2 -6' \
    -cf "$FILES/gdb-$GDB_VERSION-linux-x64.tar.xz" -C "$STAGE" "$GDB_ROOT"

log "building bookworm-compatible binutils 2.47 binaries in a Debian bookworm container"
BINUTILS_ROOT=binutils-2.47-x86_64-linux
rm -rf "$WORK/build"; mkdir -p "$WORK/build/obj" "$WORK/build/stage" "$STAGE/$BINUTILS_ROOT"/{bin,lib,libexec}
tar -xf "$FILES/binutils-2.47-source.tar.xz" -C "$WORK/build"
cat > "$WORK/build-binutils.sh" <<'EOS'
#!/bin/bash
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y --no-install-recommends build-essential xz-utils tar findutils sed coreutils >/dev/null
src=$(find /work/build -maxdepth 1 -type d -name 'binutils-*' | head -1)
[ -n "$src" ] || { echo 'binutils source tree missing' >&2; exit 1; }
mkdir -p /work/build/obj /work/build/stage
cd /work/build/obj
"$src/configure" --prefix=/usr --disable-nls --disable-werror --disable-gdb \
    --disable-sim --disable-gprofng --disable-gprof --disable-plugins \
    --without-zstd --enable-host-shared=no CFLAGS='-O2' >/dev/null
make -j2 MAKEINFO=true all-gas all-ld all-binutils >/dev/null
make MAKEINFO=true install-gas install-ld install-binutils DESTDIR=/work/build/stage >/dev/null
EOS
chmod +x "$WORK/build-binutils.sh"
docker run --rm -v "$WORK:/work" -w /work debian:bookworm bash /work/build-binutils.sh
sudo chown -R "$(id -u):$(id -g)" "$WORK/build/stage"
cp -a "$WORK/build/stage/usr/bin/." "$STAGE/$BINUTILS_ROOT/libexec/"
LDSCRIPTS=$(find "$WORK/build/stage/usr" -type d -name ldscripts -print -quit)
[ -n "$LDSCRIPTS" ] || die "binutils ldscripts directory missing"
cp -a "$LDSCRIPTS" "$STAGE/$BINUTILS_ROOT/lib/"
for f in "$STAGE/$BINUTILS_ROOT/libexec/"*; do
    [ -f "$f" ] || continue
    name=$(basename "$f")
    extra=""
    case $name in
        ld|ld.bfd) extra='-L"$here/lib"' ;;
    esac
    cat > "$STAGE/$BINUTILS_ROOT/bin/$name" <<'WRAP'
#!/bin/sh
here=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
export LIBRARY_PATH="$here/lib${LIBRARY_PATH:+:$LIBRARY_PATH}"
export COMPILER_PATH="$here/libexec${COMPILER_PATH:+:$COMPILER_PATH}"
exec "$here/libexec/__NAME__" __EXTRA__ "$@"
WRAP
    sed -i "s|__NAME__|$name|; s|__EXTRA__|$extra|" "$STAGE/$BINUTILS_ROOT/bin/$name"
    chmod 755 "$STAGE/$BINUTILS_ROOT/bin/$name"
done
cat > "$STAGE/$BINUTILS_ROOT/PIN" <<'PIN'
binutils 2.47 (2.47.20260726) built once from the pinned GNU source tarball
inside a Debian bookworm container (glibc 2.36) for broad Linux compatibility
PIN
tar --sort=name --mtime=@0 --owner=0 --group=0 --numeric-owner -I 'xz -T2 -6' \
    -cf "$FILES/binutils-2.47-x86_64-linux.tar.xz" -C "$STAGE" "$BINUTILS_ROOT"

# Every QEMU field is read out of scripts/ensure_qemu_user.sh, the script that
# enforces the pin at install time. A second copy of the version/digest here is
# exactly how this cache came to hold mold 2.42.1 for three weeks.
QEMU_VERSION=$(sed -n 's/^QEMU_VERSION=\([0-9.]*\)$/\1/p' scripts/ensure_qemu_user.sh)
QEMU_DEB_VERSION=$(sed -n 's/^DEB_VERSION=\(.*\)$/\1/p' scripts/ensure_qemu_user.sh)
QEMU_SHA=$(sed -n 's/^DEB_SHA256=\([0-9a-f]\{64\}\)$/\1/p' scripts/ensure_qemu_user.sh)
QEMU_SHA1=$(sed -n 's/^DEB_SHA1=\([0-9a-f]\{40\}\)$/\1/p' scripts/ensure_qemu_user.sh)
[ -n "$QEMU_VERSION" ] && [ -n "$QEMU_DEB_VERSION" ] && [ -n "$QEMU_SHA" ] && [ -n "$QEMU_SHA1" ] \
    || die "cannot read the QEMU pins from scripts/ensure_qemu_user.sh"
QEMU_DEB="qemu-user_${QEMU_DEB_VERSION}_amd64.deb"

log "downloading QEMU user $QEMU_VERSION from Debian sid"
fetch "$SRC/$QEMU_DEB" "https://deb.debian.org/debian/pool/main/q/qemu/$QEMU_DEB" || \
    fetch "$SRC/$QEMU_DEB" "https://snapshot.debian.org/file/$QEMU_SHA1"
verify "$SRC/$QEMU_DEB" "$QEMU_SHA"
: > "$FILES/debian-debs.sha256"
printf '%s  %s (Debian qemu-user %s)\n' "$QEMU_SHA" "$QEMU_DEB" "$QEMU_DEB_VERSION" >> "$FILES/debian-debs.sha256"
rm -rf "$EXTRACT/qemu"; mkdir -p "$EXTRACT/qemu"
dpkg-deb -x "$SRC/$QEMU_DEB" "$EXTRACT/qemu"
QEMU_ROOT="qemu-user-$QEMU_VERSION-x86_64"
rm -rf "$STAGE/$QEMU_ROOT"; mkdir -p "$STAGE/$QEMU_ROOT/bin"
for a in aarch64 arm i386 riscv64; do
    install -m755 "$EXTRACT/qemu/usr/bin/qemu-$a" "$STAGE/$QEMU_ROOT/bin/qemu-$a"
    "$STAGE/$QEMU_ROOT/bin/qemu-$a" --version | head -1 | grep -F "$QEMU_VERSION" >/dev/null \
        || die "qemu-$a is not $QEMU_VERSION"
done
printf '%s  %s\n' "$QEMU_SHA" "$QEMU_DEB (Debian qemu-user $QEMU_DEB_VERSION)" > "$STAGE/$QEMU_ROOT/PIN"
tar --sort=name --mtime=@0 --owner=0 --group=0 --numeric-owner -I 'xz -T2 -6' \
    -cf "$FILES/$QEMU_ROOT.tar.xz" -C "$STAGE" "$QEMU_ROOT"

# The kver, the source tree and the six digests are all read from
# tests/linker/setup_kernel_tools.sh -- the script whose gate verifies the very
# same sources. The publisher used to carry a second copy of the pin table and
# of the (torvalds) tree URL, which cannot survive a stable-release bump:
# v7.3.1 does not exist in the torvalds tree at all.
KVER=$(sed -n 's/^PINNED_KVER=\(v[0-9.]*\)$/\1/p' tests/linker/setup_kernel_tools.sh)
KBASE=$(sed -n 's|^BASE="\(https://[^"]*\)"$|\1|p' tests/linker/setup_kernel_tools.sh)
[ -n "$KVER" ] && [ -n "$KBASE" ] || die "cannot read the kernel relocs pins from tests/linker/setup_kernel_tools.sh"

log "downloading and building kernel relocs $KVER"
KREL_ROOT="kernel-relocs-$KVER-x86_64"
rm -rf "$STAGE/$KREL_ROOT"; mkdir -p "$STAGE/$KREL_ROOT"/{bin,src/tools}
# Emit "<file>\t<sha256>" straight from the gate's PINS definition, so the two
# can never disagree about what "the pinned sources" means.
while IFS=$'\t' read -r kfile ksha; do
    [ -n "$kfile" ] || continue
    if [[ $kfile == tools/* ]]; then url="$KBASE/tools/include/$kfile?h=$KVER"; else url="$KBASE/arch/x86/tools/$kfile?h=$KVER"; fi
    fetch "$STAGE/$KREL_ROOT/src/$kfile" "$url"
    verify "$STAGE/$KREL_ROOT/src/$kfile" "$ksha"
done < <(python3 - <<'PYK'
import pathlib, re
text = pathlib.Path('tests/linker/setup_kernel_tools.sh').read_text()
pins = re.search(r"^PINS='(.*?)'$", text, re.M | re.S).group(1)
for line in pins.splitlines():
    sha, _, name = line.strip().partition('  ')
    if sha and name:
        print(f"{name}\t{sha}")
PYK
)
( cd "$STAGE/$KREL_ROOT/src" && cc -O2 -I. -o ../bin/relocs relocs_common.c relocs_32.c relocs_64.c )
printf '%s sha256:digested-by-tag-manifest\n' "$KVER" > "$STAGE/$KREL_ROOT/STAMP"
tar --sort=name --mtime=@0 --owner=0 --group=0 --numeric-owner -I 'xz -T2 -6' \
    -cf "$FILES/$KREL_ROOT.tar.xz" -C "$STAGE" "$KREL_ROOT"

log "functional smoke test: GCC 16.2 + Debian binutils 2.47 + mold 3.0.0 + Clang $CLANG_VERSION + GDB $GDB_VERSION + QEMU i386"
mkdir -p "$EXTRACT/tools"
tar -xf "$FILES/gcc-16.2.0-bookworm-x86_64.tar.xz" -C "$EXTRACT/tools"
tar -xf "$FILES/clang-$CLANG_VERSION-linux-x64.tar.xz" -C "$EXTRACT/tools"
tar -xf "$FILES/gdb-$GDB_VERSION-linux-x64.tar.xz" -C "$EXTRACT/tools"
tar -xf "$FILES/rust-1.99.1-x86_64-unknown-linux-gnu.tar.xz" -C "$EXTRACT/tools"
tar -xf "$FILES/mold-3.0.0-x86_64-linux.tar.gz" -C "$EXTRACT/tools"
tar -xf "$FILES/binutils-2.47-x86_64-linux.tar.xz" -C "$EXTRACT/tools"
GCC_BIN=$(find "$EXTRACT/tools" -type f -name gcc-16 -perm -111 | head -1)
CLANG_BIN=$(find "$EXTRACT/tools" -type f \( -name 'clang-23*' -o -name 'clang' \) -perm -111 | head -1)
RUSTC_BIN=$(find "$EXTRACT/tools" -type f -name rustc -perm -111 | head -1)
MOLD_BIN=$(find "$EXTRACT/tools" -type f -name mold -perm -111 | head -1)
GDB_BIN=$(find "$EXTRACT/tools" -type f -name gdb -perm -111 | head -1)
[ -n "$GCC_BIN" ] && [ -n "$CLANG_BIN" ] && [ -n "$RUSTC_BIN" ] && [ -n "$MOLD_BIN" ] && [ -n "$GDB_BIN" ] || die "tool binaries missing from archives"
"$GCC_BIN" --version | head -1
"$CLANG_BIN" --version | head -1
"$RUSTC_BIN" --version
"$MOLD_BIN" --version
"$GDB_BIN" --version | head -1 | grep -Fx "GNU gdb (GDB) $GDB_VERSION" >/dev/null \
    || die "cached GDB is not the pinned $GDB_VERSION oracle"
BI="$EXTRACT/tools/$BINUTILS_ROOT"
ALL_LIBS=$(find "$EXTRACT/tools" -type d \( -name lib -o -name lib64 -o -path '*/usr/lib/x86_64-linux-gnu' \) -print | paste -sd: -)
export LD_LIBRARY_PATH="$ALL_LIBS${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
"$BI/bin/as" --version | head -1
"$BI/bin/ld" --version | head -1
"$BI/bin/objdump" --version | head -1
"$BI/bin/readelf" --version | head -1
cat > "$TEST/probe.c" <<'C'
int main(void) { return 42; }
C
"$GCC_BIN" -B"$BI/bin" "$TEST/probe.c" -o "$TEST/probe-gcc"
"$TEST/probe-gcc" || [ $? = 42 ] || die "gcc+binutils probe did not return 42"
PATH="$(dirname "$MOLD_BIN"):$PATH" "$CLANG_BIN" -fuse-ld=mold -B"$(dirname "$MOLD_BIN")" "$TEST/probe.c" -o "$TEST/probe-clang"
"$TEST/probe-clang" || [ $? = 42 ] || die "clang+mold probe did not return 42"
cat > "$TEST/probe32.c" <<'C'
__attribute__((noreturn)) void _start(void) {
    __asm__ volatile("int $0x80" :: "a"(1), "b"(42));
    __builtin_unreachable();
}
C
"$GCC_BIN" -m32 -nostdlib -static -no-pie -fno-pie -Wl,-e,_start -B"$BI/bin" "$TEST/probe32.c" -o "$TEST/probe32"
"$STAGE/$QEMU_ROOT/bin/qemu-i386" "$TEST/probe32" || [ $? = 42 ] || die "qemu-i386 did not run the 32-bit probe"

log "writing metadata"
python3 - "$FILES" "$TAG" "$STAGE/$KREL_ROOT" "$CLANG_VERSION" "$QEMU_VERSION" "$KVER" "$GDB_VERSION" <<'PY'
import hashlib, json, os, sys
from pathlib import Path
files=Path(sys.argv[1]); tag=sys.argv[2]; clang_version=sys.argv[4]
qemu_version=sys.argv[5]; kver=sys.argv[6].removeprefix('v')
gdb_version=sys.argv[7]
def digest(p):
    with open(p,'rb') as f: return hashlib.file_digest(f,'sha256').hexdigest()
tools={}
def add(name,version,fn,root=None,kind='tar.xz'):
    p=files/fn
    tools[name]={'version':version,'file':fn,'sha256':digest(p),'size':p.stat().st_size,'root':root,'kind':kind}
def top(fn):
    import subprocess
    out=subprocess.check_output(['tar','-tf',str(files/fn)],text=True).splitlines()
    return out[0].lstrip('./').split('/',1)[0]
for name,ver,fn in [
 ('gcc','16.2.0','gcc-16.2.0-bookworm-x86_64.tar.xz'),
 ('debs-bookworm','bookworm','debs-bookworm.tar.xz'),
 ('binutils-source','2.47','binutils-2.47-source.tar.xz'),
 ('arm-a64-xml','2026-09','ISA_A64_xml_A_profile-2026-09_md.tar.gz'),
 ('clang',clang_version,f'clang-{clang_version}-linux-x64.tar.xz'),
 ('gdb',gdb_version,f'gdb-{gdb_version}-linux-x64.tar.xz'),
 ('rust','1.99.1','rust-1.99.1-x86_64-unknown-linux-gnu.tar.xz'),
 ('mold','3.0.0','mold-3.0.0-x86_64-linux.tar.gz'),
 ('binutils','2.47','binutils-2.47-x86_64-linux.tar.xz'),
 ('qemu',qemu_version,f'qemu-user-{qemu_version}-x86_64.tar.xz'),
 ('kernel-relocs',kver,f'kernel-relocs-v{kver}-x86_64.tar.xz')]:
    add(name,ver,fn,top(fn),'tar.gz' if fn.endswith('.gz') else 'tar.xz')
cache={'schema':1,'repository':'ms178/lccc','tag':tag,'part_size':90*1024*1024,
       'tools':tools,'priority':['rust','clang','gcc','gdb','binutils','mold','qemu','kernel-relocs','arm-a64-xml','debs-bookworm','binutils-source'],
       'provenance':{'debian_sid_packages':'debian-debs.sha256 (QEMU user)','binutils':'one-time build from pinned 2.47 source in Debian bookworm container','kernel':f'v{kver} stable kernel.org tree (sources pinned by tests/linker/setup_kernel_tools.sh)','mold':'rui314/mold v3.0.0 release','clang':f'official LLVM {clang_version} asset, staged by scripts/ci_install_clang_oracle.sh','gdb':f'GNU GDB {gdb_version} built with -march=haswell by scripts/ci_install_gdb_oracle.sh','rust':'harness-cache-4 release asset'}}
(files/'CACHE.json').write_text(json.dumps(cache,indent=2,sort_keys=True)+'\n')
(files/'MANIFEST.sha256').write_text(''.join(f"{v['sha256']}  {v['file']}\n" for _,v in sorted(tools.items())) + f"{digest(files/'CACHE.json')}  CACHE.json\n")
(files/'README.md').write_text(f'''# {tag}\n\nGitHub-only toolchain cache for network-constrained agents.\n\n```\ngit fetch origin tag {tag}\npython3 scripts/restore_agent_toolchain_cache.py --tag {tag} --prefix /path/to/toolchains\n. /path/to/toolchains/env.sh\n```\n\nEvery file is split into <90 MiB parts and verified against MANIFEST.sha256.\nCACHE.json records the tool versions, roots, sizes and digests.\n''')
PY

# A cache is an input contract, not a scrapheap. Refuse to publish any
# retired toolchain: the MSRV floor, the superseded Clang/QEMU/relocs payloads.
python3 - "$FILES/CACHE.json" "$FILES" <<'PY'
import json, sys
from pathlib import Path
cache=json.loads(Path(sys.argv[1]).read_text()); files=Path(sys.argv[2])
banned_names={'rust-msrv','clang-tag-tree'}
bad=[n for n in list(cache['tools'])+cache.get('priority',[]) if n in banned_names]
obsolete=('1.97.0','23.1.4','11.1.2','v7.3-x86_64','2.42.1')
for name,spec in cache['tools'].items():
    if any(s in spec['file'] for s in obsolete):
        bad.append(f"{name}={spec['file']}")
for path in files.iterdir():
    if path.is_file() and any(s in path.name for s in obsolete):
        bad.append(path.name)
if bad:
    sys.exit('refusing to publish obsolete cache payloads: '+', '.join(sorted(bad)))
print('cache content guard: current tools only, no obsolete payloads')
PY

log "splitting payloads into <90 MiB parts"
rm -rf "$TREE"; mkdir -p "$TREE"
python3 - "$FILES" "$TREE" "$PART_SIZE" <<'PY'
import sys
from pathlib import Path
files=Path(sys.argv[1]); tree=Path(sys.argv[2]); size=int(sys.argv[3])
names=['MANIFEST.sha256','CACHE.json','README.md','debian-debs.sha256'] + sorted(p.name for p in files.iterdir() if p.is_file() and p.name not in {'MANIFEST.sha256','CACHE.json','README.md','debian-debs.sha256'})
parts={}
for name in names:
    p=files/name
    n=0
    with p.open('rb') as src:
        while True:
            chunk=src.read(size)
            if not chunk: break
            (tree/f'{name}.part{n:02d}').write_bytes(chunk)
            n+=1
    parts[name]=n
parts['PARTS.tsv']=1
parts_text=''.join(f'{n}\t{name}\n' for name,n in parts.items())
(tree/'PARTS.tsv').write_text(parts_text)
(tree/'PARTS.tsv.part00').write_text(parts_text)
PY

log "creating immutable tag $TAG"
if [ -n "$(git ls-remote --tags "$REMOTE" "refs/tags/$TAG")" ]; then die "tag $TAG already exists; choose a new tag"; fi
: > "$TREE/tree.txt"
for p in "$TREE"/*.part*; do
    [ -e "$p" ] || continue
    [ "$(basename "$p")" = 'tree.txt' ] && continue
    sha=$(git hash-object -w "$p")
    printf '100644 blob %s\t%s\n' "$sha" "$(basename "$p")" >> "$TREE/tree.txt"
done
tree_sha=$(git mktree < "$TREE/tree.txt")
commit_sha=$(git -c user.name='lccc-agent-cache' -c user.email='agent-cache@users.noreply.github.com' \
    commit-tree "$tree_sha" -m "GitHub-only agent toolchain cache $TAG")
git -c user.name='lccc-agent-cache' -c user.email='agent-cache@users.noreply.github.com' \
    tag -a "$TAG" "$commit_sha" -m "GitHub-only agent toolchain cache $TAG"
TAG_SHA=$(git rev-parse "$TAG")
if [ "$PUSH" = 1 ]; then
    git push "$REMOTE" "refs/tags/$TAG"
else
    log "PUSH=0: tag created locally only"
fi
mkdir -p "$WORK/artifact"
cp "$FILES/CACHE.json" "$WORK/artifact/agent-cache-cache.json"
cp "$FILES/MANIFEST.sha256" "$WORK/artifact/MANIFEST.sha256"
python3 - "$WORK/artifact/agent-cache-lock.json" "$FILES/CACHE.json" "$TAG" "$TAG_SHA" "$commit_sha" "$tree_sha" <<'PY'
import hashlib,json,sys
from pathlib import Path
out=Path(sys.argv[1]); cache=json.loads(Path(sys.argv[2]).read_text())
cache['tag']=sys.argv[3]; cache['tag_sha']=sys.argv[4]; cache['commit_sha']=sys.argv[5]; cache['tree_sha']=sys.argv[6]
with open(sys.argv[2],'rb') as f: cache['cache_json_sha256']=hashlib.file_digest(f,'sha256').hexdigest()
out.write_text(json.dumps(cache,indent=2,sort_keys=True)+'\n')
PY
log "tag=$TAG tag_sha=$TAG_SHA commit=$commit_sha tree=$tree_sha"
log "artifacts: $WORK/artifact"
