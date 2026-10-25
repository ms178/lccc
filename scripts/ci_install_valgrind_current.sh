#!/usr/bin/env bash
# Install the current Valgrind/Callgrind release in a private, verified prefix.
# Source: the VALGRIND_3_28_0 tag mirrored from upstream, pinned to its commit
# and codeload archive digest. No distribution Valgrind or compiler fallback.
set -Eeuo pipefail

version=3.28.0
commit=5a4657de12a8ed16b07175e1769df4dad31f80f2
digest=3e6c1aa8c8bbf14cbc8c70a0eb1c7bb40b8515c8f93e23191c8ceffeadab91fa
prefix=${LCCC_VALGRIND_PREFIX:-${XDG_CACHE_HOME:-$HOME/.cache}/lccc/valgrind/$version}
print_bin=0
while (( $# )); do
    case $1 in
        --prefix) prefix=${2:?--prefix requires a directory}; shift 2 ;;
        --print-bin) print_bin=1; shift ;;
        -h|--help) sed -n '2,4p' "$0"; exit 0 ;;
        *) echo "valgrind-current: unknown argument: $1" >&2; exit 2 ;;
    esac
done

fail() { echo "valgrind-current: $*" >&2; exit 1; }
log() { echo "valgrind-current: $*" >&2; }
[[ $prefix = /* && $prefix != / && $prefix != "$HOME" ]] || fail "prefix must be an absolute private directory"

verify() {
    [[ -f $prefix/.lccc-source && -f $prefix/.lccc-tools.sha256 ]] || return 1
    [[ -x $prefix/libexec/valgrind/callgrind-amd64-linux && -x $prefix/libexec/valgrind/callgrind-x86-linux ]] || return 1
    [[ $(cat "$prefix/.lccc-source") = "$commit $digest" ]] || return 1
    (cd "$prefix" && sha256sum --status -c .lccc-tools.sha256) || return 1
    [[ $("$prefix/bin/valgrind" --version) = "valgrind-$version" ]] || return 1
}

publish() {
    if [[ -n ${GITHUB_PATH:-} ]]; then printf '%s\n' "$prefix/bin" >> "$GITHUB_PATH"; fi
    if (( print_bin )); then printf '%s\n' "$prefix/bin"; fi
}

if verify; then
    log "reusing verified Valgrind $version at $prefix"
    publish
    exit 0
fi
# Do not overlay a stale or foreign installation, or recursively remove it.
# A failed build is explicit and cannot turn into a version-only cache hit.
if [[ -d $prefix ]] && [[ -n $(find "$prefix" -mindepth 1 -print -quit) ]]; then
    fail "unverified nonempty prefix: $prefix; choose a new private prefix or explicitly remove the failed installation"
fi

export CC=${CC:-gcc-16}
export CXX=${CXX:-g++-16}
for compiler in "$CC" "$CXX"; do
    command -v "$compiler" >/dev/null || fail "missing compiler: $compiler"
    [[ $("$compiler" -dumpfullversion) = 16.2.0 ]] || fail "$compiler must be GCC 16.2.0"
done
# The research lane puts the authenticated GAS pair and current mold on PATH.
# Reject old assembler/linker defaults before a build can hide their use.
as_version=$(as --version | sed -n '1p')
[[ $as_version =~ [[:space:]]2\.47(\.[0-9]+)?$ ]] || fail "GAS 2.47 is required: $as_version"
ld_version=$(ld --version | sed -n '1p')
[[ $ld_version = 'mold 2.42.1 '* ]] || fail "mold 2.42.1 is required: $ld_version"
for tool in curl tar sha256sum autoconf autoheader automake aclocal make; do
    command -v "$tool" >/dev/null || fail "missing build dependency: $tool"
done

work=$(mktemp -d "${TMPDIR:-/tmp}/valgrind-current.XXXXXX")
trap 'rm -rf "$work"' EXIT
curl --fail --location --retry 3 --retry-all-errors \
    "https://codeload.github.com/PragsPud/valgrind-git-mirror/tar.gz/$commit" \
    --output "$work/source.tar.gz" >&2
printf '%s  %s\n' "$digest" "$work/source.tar.gz" | sha256sum -c - >&2
mkdir "$work/source" "$work/build"
tar -xzf "$work/source.tar.gz" -C "$work/source" --strip-components=1
(
    cd "$work/source"
    ./autogen.sh
    cd "$work/build"
    "$work/source/configure" --prefix="$prefix"
    # Build both amd64 and i386 tools; verify their presence below. Never run
    # concurrent compiler/tool builds on the constrained research machine.
    make -j2
    make install
) >&2
[[ $("$prefix/bin/valgrind" --version) = "valgrind-$version" ]] || fail "installed version mismatch"
"$prefix/bin/valgrind" --tool=callgrind --error-exitcode=97 \
    --callgrind-out-file="$work/callgrind.out" /bin/true >&2
[[ -x $prefix/libexec/valgrind/callgrind-x86-linux ]] || fail "i386 Callgrind was not built; install multilib development files"
cat > "$work/probe32.s" <<'ASM'
.text
.globl _start
_start:
    movl $1, %eax
    xorl %ebx, %ebx
    int $0x80
ASM
as --32 "$work/probe32.s" -o "$work/probe32.o"
ld -m elf_i386 -static "$work/probe32.o" -o "$work/probe32"
"$prefix/bin/valgrind" --tool=callgrind --error-exitcode=97 \
    --callgrind-out-file="$work/callgrind32.out" "$work/probe32" >&2
# Bind the actual tools, not merely a version string, to this source receipt.
(
    cd "$prefix"
    find bin libexec -type f -print0 | sort -z | xargs -0 sha256sum > .lccc-tools.sha256
)
printf '%s %s\n' "$commit" "$digest" > "$prefix/.lccc-source"
verify || fail "installed tool receipt verification failed"
log "installed verified Valgrind $version at $prefix"
publish
