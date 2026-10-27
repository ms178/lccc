#!/usr/bin/env bash
# Publish an immutable, GitHub-only toolchain cache tag for network-constrained
# agents.  Everything in the tag is reachable by `git fetch origin tag ...`;
# no release-assets CDN, Debian mirror, kernel.org or Arm CDN is needed by the
# consumer.  Run this on a host with full egress (normally a workflow_dispatch
# job); all bytes are verified before they enter the tag.
set -Eeuo pipefail

TAG=${TAG:-harness-cache-5}
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

need git curl tar xz sha256sum split dpkg-deb python3 gh gcc
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
for t in harness-cache-1 harness-cache-2 harness-cache-3 harness-cache-4 rust-cache-1.97.0; do
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

log "materialising the 1.97.0 MSRV Rust components"
MSRV="$STAGE/rust-1.97.0-msrv"
mkdir -p "$MSRV"
for comp in rustc cargo rust-std clippy rustfmt; do
    member="$comp-1.97.0-x86_64-unknown-linux-gnu.tar.xz"
    blob rust-cache-1.97.0 "$member.part00" "$MSRV/$member"
    verify "$MSRV/$member" "$(git show rust-cache-1.97.0:MANIFEST.sha256 | awk -v m="$member" '$2==m{print $1}')"
done
tar --sort=name --mtime=@0 --owner=0 --group=0 --numeric-owner -I 'xz -T2 -6' \
    -cf "$FILES/rust-1.97.0-msrv.tar.xz" -C "$STAGE" rust-1.97.0-msrv

log "downloading locked Clang 23.1.4 and Rust 1.99.1 release assets"
CLANG_SHA=$(python3 -c 'import json;print(json.load(open("engineering/toolchains/cache-lock.json"))["tools"]["clang"]["sha256"])')
RUST_SHA=$(python3 -c 'import json;print(json.load(open("engineering/toolchains/cache-lock.json"))["tools"]["rust"]["sha256"])')
gh release download harness-cache-4 --repo "$REPO" --pattern 'clang-23.1.4-*.tar.xz' --dir "$SRC" --clobber
gh release download harness-cache-4 --repo "$REPO" --pattern 'rust-1.99.1-*.tar.xz' --dir "$SRC" --clobber
clang_asset=$(find "$SRC" -maxdepth 1 -name 'clang-23.1.4-*.tar.xz' -print -quit)
rust_asset=$(find "$SRC" -maxdepth 1 -name 'rust-1.99.1-*.tar.xz' -print -quit)
[ -n "$clang_asset" ] && [ -n "$rust_asset" ] || die "locked release assets not downloaded"
verify "$clang_asset" "$CLANG_SHA"; verify "$rust_asset" "$RUST_SHA"
cp "$clang_asset" "$FILES/clang-23.1.4-linux-x64.tar.xz"
cp "$rust_asset" "$FILES/rust-1.99.1-x86_64-unknown-linux-gnu.tar.xz"

log "downloading mold 3.0.0 from its pinned upstream release"
MOLD_SHA=$(sed -n 's/^SHA256=\([0-9a-f]\{64\}\)$/\1/p' scripts/ci_install_mold_oracle.sh)
fetch "$SRC/mold-3.0.0-x86_64-linux.tar.gz" https://github.com/rui314/mold/releases/download/v3.0.0/mold-3.0.0-x86_64-linux.tar.gz
verify "$SRC/mold-3.0.0-x86_64-linux.tar.gz" "$MOLD_SHA"
cp "$SRC/mold-3.0.0-x86_64-linux.tar.gz" "$FILES/mold-3.0.0-x86_64-linux.tar.gz"

log "downloading Debian sid binutils 2.47 binaries and runtime dependencies"
fetch "$SRC/Packages.xz" https://deb.debian.org/debian/dists/sid/main/binary-amd64/Packages.xz
xz -dc "$SRC/Packages.xz" > "$SRC/Packages"
python3 - "$SRC/Packages" "$SRC/deb-list.tsv" <<'PY'
import sys
from pathlib import Path
text=Path(sys.argv[1]).read_text(errors='ignore').split('\n\n')
want={'binutils-x86-64-linux-gnu':'2.47.50.20261021-1','binutils-common':'2.47.50.20261021-1',
      'libbinutils':'2.47.50.20261021-1','libctf-nobfd0':None,'libctf0':None,
      'libjansson4':None,'libsframe3':None,'libzstd1':None,'zlib1g':None}
best={}
for stanza in text:
    d={}
    for line in stanza.splitlines():
        if ':' in line and not line.startswith(' '):
            k,v=line.split(':',1); d[k]=v.strip()
    p=d.get('Package')
    if p not in want or 'Filename' not in d or 'SHA256' not in d: continue
    v=d.get('Version','')
    if want[p] and v != want[p]: continue
    key=(p,v)
    best[p]=(v,d['Filename'],d['SHA256'])
missing=set(want)-set(best)
if missing: raise SystemExit(f'missing Debian packages: {sorted(missing)}')
with open(sys.argv[2],'w') as out:
    for p,(v,f,s) in sorted(best.items()): out.write(f'{p}\t{v}\t{f}\t{s}\n')
PY
rm -rf "$EXTRACT/binutils"; mkdir -p "$EXTRACT/binutils"
: > "$FILES/debian-debs.sha256"
while IFS=$'\t' read -r pkg ver filename sha; do
    deb="$SRC/$(basename "$filename")"
    fetch "$deb" "https://deb.debian.org/debian/$filename"
    verify "$deb" "$sha"
    printf '%s  %s (%s %s)\n' "$sha" "$(basename "$filename")" "$pkg" "$ver" >> "$FILES/debian-debs.sha256"
    dpkg-deb -x "$deb" "$EXTRACT/binutils"
done < "$SRC/deb-list.tsv"

BINUTILS_ROOT=binutils-2.47-debian-sid-x86_64
rm -rf "$STAGE/$BINUTILS_ROOT"; mkdir -p "$STAGE/$BINUTILS_ROOT"/{bin,usr/bin,usr/lib,usr/lib/x86_64-linux-gnu}
cp -a "$EXTRACT/binutils/usr/bin/." "$STAGE/$BINUTILS_ROOT/usr/bin/"
cp -a "$EXTRACT/binutils/usr/lib/x86_64-linux-gnu/." "$STAGE/$BINUTILS_ROOT/usr/lib/x86_64-linux-gnu/" 2>/dev/null || true
cp -a "$EXTRACT/binutils/usr/lib/." "$STAGE/$BINUTILS_ROOT/usr/lib/" 2>/dev/null || true
for f in "$STAGE/$BINUTILS_ROOT/usr/bin/"*; do
    [ -e "$f" ] || continue
    [ -d "$f" ] && continue
    name=$(basename "$f")
    cat > "$STAGE/$BINUTILS_ROOT/bin/$name" <<WRAP
#!/bin/sh
here=\$(CDPATH= cd -- "\$(dirname "\$0")/.." && pwd)
export LD_LIBRARY_PATH="\$here/usr/lib/x86_64-linux-gnu:\$here/usr/lib\${LD_LIBRARY_PATH:+:\$LD_LIBRARY_PATH}"
exec "\$here/usr/bin/$name" "\$@"
WRAP
    chmod 755 "$STAGE/$BINUTILS_ROOT/bin/$name"
done
cat > "$STAGE/$BINUTILS_ROOT/PIN" <<'PIN'
binutils 2.47.50.20261021-1 Debian sid x86_64-linux-gnu prebuilt binaries
PIN
tar --sort=name --mtime=@0 --owner=0 --group=0 --numeric-owner -I 'xz -T2 -6' \
    -cf "$FILES/binutils-2.47-debian-sid-x86_64.tar.xz" -C "$STAGE" "$BINUTILS_ROOT"

log "downloading QEMU user 11.1.2 from Debian sid"
QEMU_SHA=$(sed -n 's/^DEB_SHA256=\([0-9a-f]\{64\}\)$/\1/p' scripts/ensure_qemu_user.sh)
QEMU_DEB=qemu-user_11.1.2+ds-3_amd64.deb
fetch "$SRC/$QEMU_DEB" "https://deb.debian.org/debian/pool/main/q/qemu/$QEMU_DEB" || \
    fetch "$SRC/$QEMU_DEB" "https://snapshot.debian.org/file/6067b061f3dd361d94e6c48a1044f79860c56c51"
verify "$SRC/$QEMU_DEB" "$QEMU_SHA"
rm -rf "$EXTRACT/qemu"; mkdir -p "$EXTRACT/qemu"
dpkg-deb -x "$SRC/$QEMU_DEB" "$EXTRACT/qemu"
QEMU_ROOT=qemu-user-11.1.2-x86_64
rm -rf "$STAGE/$QEMU_ROOT"; mkdir -p "$STAGE/$QEMU_ROOT/bin"
for a in aarch64 arm i386 riscv64; do
    install -m755 "$EXTRACT/qemu/usr/bin/qemu-$a" "$STAGE/$QEMU_ROOT/bin/qemu-$a"
    "$STAGE/$QEMU_ROOT/bin/qemu-$a" --version | head -1 | grep -F '11.1.2' >/dev/null || die "qemu-$a is not 11.1.2"
done
printf '%s  %s\n' "$QEMU_SHA" "$QEMU_DEB (Debian qemu-user 11.1.2+ds-3)" > "$STAGE/$QEMU_ROOT/PIN"
tar --sort=name --mtime=@0 --owner=0 --group=0 --numeric-owner -I 'xz -T2 -6' \
    -cf "$FILES/qemu-user-11.1.2-x86_64.tar.xz" -C "$STAGE" "$QEMU_ROOT"

log "downloading and building kernel relocs v7.3"
KREL_ROOT=kernel-relocs-v7.3-x86_64
rm -rf "$STAGE/$KREL_ROOT"; mkdir -p "$STAGE/$KREL_ROOT"/{bin,src/tools}
BASE=https://git.kernel.org/pub/scm/linux/kernel/git/torvalds/linux.git/plain
declare -A KPINS=(
    [relocs.c]=acbeca7b2fb6f84502c96385edf117f5170756d743373a7a581b5b46427803f7
    [relocs.h]=61783eeddcf582c6f8c74c63f8f5170df51e34819a8b8dad9af68117f5c185d0
    [relocs_common.c]=a49f30d39b362c29bdd27f19cadffdcfefefe5ad35511dd4b8f00d002a1cf08f
    [relocs_32.c]=ae8b4ffcfe0367bb457a73b9a9534bf2ec495e2f7d99e23b1257b0009d693680
    [relocs_64.c]=88e45459755585c934a9e24ebb9f6b71fd27f8d10c2fa9cc4a4d51fe3719166b
    [tools/le_byteshift.h]=87b40e54fed5b27a9acd784e41521d8bbf95922448d07bea17be6b4bb5fa7925
)
for f in "${!KPINS[@]}"; do
    if [[ $f == tools/* ]]; then url="$BASE/tools/include/$f?h=v7.3"; else url="$BASE/arch/x86/tools/$f?h=v7.3"; fi
    fetch "$STAGE/$KREL_ROOT/src/$f" "$url"
    verify "$STAGE/$KREL_ROOT/src/$f" "${KPINS[$f]}"
done
( cd "$STAGE/$KREL_ROOT/src" && cc -O2 -I. -o ../../bin/relocs relocs_common.c relocs_32.c relocs_64.c )
printf 'v7.3 sha256:digested-by-tag-manifest\n' > "$STAGE/$KREL_ROOT/STAMP"
tar --sort=name --mtime=@0 --owner=0 --group=0 --numeric-owner -I 'xz -T2 -6' \
    -cf "$FILES/kernel-relocs-v7.3-x86_64.tar.xz" -C "$STAGE" "$KREL_ROOT"

log "functional smoke test: GCC 16.2 + Debian binutils 2.47 + mold 3.0.0 + Clang 23.1.4 + QEMU i386"
mkdir -p "$EXTRACT/tools"
tar -xf "$FILES/gcc-16.2.0-bookworm-x86_64.tar.xz" -C "$EXTRACT/tools"
tar -xf "$FILES/clang-23.1.4-linux-x64.tar.xz" -C "$EXTRACT/tools"
tar -xf "$FILES/rust-1.99.1-x86_64-unknown-linux-gnu.tar.xz" -C "$EXTRACT/tools"
tar -xf "$FILES/mold-3.0.0-x86_64-linux.tar.gz" -C "$EXTRACT/tools"
tar -xf "$FILES/binutils-2.47-debian-sid-x86_64.tar.xz" -C "$EXTRACT/tools"
GCC_BIN=$(find "$EXTRACT/tools" -type f -name gcc-16 -perm -111 | head -1)
CLANG_BIN=$(find "$EXTRACT/tools" -type f -name 'clang-23*' -perm -111 | head -1)
RUSTC_BIN=$(find "$EXTRACT/tools" -type f -name rustc -perm -111 | head -1)
MOLD_BIN=$(find "$EXTRACT/tools" -type f -name mold -perm -111 | head -1)
[ -n "$GCC_BIN" ] && [ -n "$CLANG_BIN" ] && [ -n "$RUSTC_BIN" ] && [ -n "$MOLD_BIN" ] || die "tool binaries missing from archives"
"$GCC_BIN" --version | head -1
"$CLANG_BIN" --version | head -1
"$RUSTC_BIN" --version
"$MOLD_BIN" --version
BI="$EXTRACT/tools/$BINUTILS_ROOT"
export LD_LIBRARY_PATH="$BI/usr/lib/x86_64-linux-gnu:$BI/usr/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
"$BI/usr/bin/as" --version | head -1
"$BI/usr/bin/ld" --version | head -1
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
python3 - "$FILES" "$TAG" "$STAGE/$KREL_ROOT" <<'PY'
import hashlib, json, sys
from pathlib import Path
files=Path(sys.argv[1]); tag=sys.argv[2]
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
 ('rust-msrv','1.97.0','rust-1.97.0-msrv.tar.xz'),
 ('clang','23.1.4','clang-23.1.4-linux-x64.tar.xz'),
 ('rust','1.99.1','rust-1.99.1-x86_64-unknown-linux-gnu.tar.xz'),
 ('mold','3.0.0','mold-3.0.0-x86_64-linux.tar.gz'),
 ('binutils','2.47.50.20261021-1','binutils-2.47-debian-sid-x86_64.tar.xz'),
 ('qemu','11.1.2','qemu-user-11.1.2-x86_64.tar.xz'),
 ('kernel-relocs','7.3','kernel-relocs-v7.3-x86_64.tar.xz')]:
    add(name,ver,fn,top(fn),'tar.gz' if fn.endswith('.gz') else 'tar.xz')
cache={'schema':1,'repository':'ms178/lccc','tag':tag,'part_size':90*1024*1024,
       'tools':tools,'priority':['rust','clang','gcc','binutils','mold','qemu','kernel-relocs','arm-a64-xml','debs-bookworm','binutils-source','rust-msrv'],
       'provenance':{'debian_sid_packages':'debian-debs.sha256','kernel':'v7.3 torvalds tree','mold':'rui314/mold v3.0.0 release','clang_rust':'harness-cache-4 release assets'}}
(files/'CACHE.json').write_text(json.dumps(cache,indent=2,sort_keys=True)+'\n')
(files/'MANIFEST.sha256').write_text(''.join(f"{v['sha256']}  {v['file']}\n" for _,v in sorted(tools.items())) + f"{digest(files/'CACHE.json')}  CACHE.json\n")
(files/'README.md').write_text(f'''# {tag}\n\nGitHub-only toolchain cache for network-constrained agents.\n\n```\ngit fetch origin tag {tag}\npython3 scripts/restore_agent_toolchain_cache.py --tag {tag} --prefix /path/to/toolchains\n. /path/to/toolchains/env.sh\n```\n\nEvery file is split into <90 MiB parts and verified against MANIFEST.sha256.\nCACHE.json records the tool versions, roots, sizes and digests.\n''')
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
