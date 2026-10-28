#!/usr/bin/env bash
# ============================================================================
# oracle_toolchains_from_tags.sh — restore GCC/binutils from the
# repository's verified transport tags and install the pinned current Clang,
# Rust and mold from their reviewed upstream/cache-lock pins.
#
# WHY THIS EXISTS
# ---------------
# Every gate in this repository that says "compare against GCC 16.2 / Clang
# 23.1.4" resolves its oracle by NAME on PATH (`gcc-16`, `clang-23`), and CI
# provisions those names from a distribution repository or a vendor PPA. A
# developer (or an agent) working on a host whose distribution does not carry
# GCC 16 has exactly two options today: skip every gate that needs the oracle
# (the failure mode `ci_install_gcc16.sh` was written to prevent) or spend a
# long time building one. Neither is acceptable when the point of the gate is
# to be run.
#
# `harness-cache-1` carries GCC/binutils, verified against its manifest.
# The historical harness-cache-4 source tree contains Clang 23.1.3, not 23.1.4.
# Rust 1.99.1 and Clang 23.1.4 use engineering/toolchains/cache-lock.json,
# whose reviewed hashes describe content-addressed release assets. No source
# tag is relabelled. mold 3.0.0 (2026-10-05) superseded the tag's 2.42.1
# binary; it is installed by scripts/ci_install_mold_oracle.sh from the
# digest-pinned upstream release instead of the stale tag payload. Online
# restoration needs GitHub's asset CDN; explicit offline verification never
# invokes a transport or writes the prefix.
#
# WHAT IT DOES NOT DO
# -------------------
# * It never silently substitutes a different compiler. Tag payloads must
#   match their recorded digest; Clang must match the official installer hash
#   and the requested version.
# * It does not build binutils from source. `--with-binutils` restores the
#   reviewed 2.47 binary set from the GitHub-only agent cache; the source
#   tarball is still verified as part of the tag promise.
# * It does not touch the crate build. Provisioning the oracles must never
#   change what `cargo build --profile fastbuild` compiles.
#
# USAGE
#   scripts/oracle_toolchains_from_tags.sh [options]
#
#     --prefix DIR      install root (default: ${LCCC_ORACLE_PREFIX:-/opt/oracles})
#     --bin-dir DIR     where the PATH shims go (default: /usr/local/bin)
#     --no-shims        install under --prefix only; print the env file path
#     --with-binutils   also build+install binutils 2.47 (slow)
#     --verify-only     check an existing install and exit
#     --offline         verify an existing install; never materialise or download
#     -h, --help        this text
#
# EXIT STATUS
#   0  every requested payload verified AND the extracted tool printed the
#      pinned version; shims (unless --no-shims) resolve on PATH
#   1  a payload was missing, its digest did not match, or the extracted tool
#      answered with the wrong version
#   2  usage error, or a required host tool is absent
#
# The script is idempotent: an existing install whose tools answer with the
# pinned versions is left untouched (and reported as reused), so calling it on
# every session start costs one `--version` per tool.
#
# Offline is explicit, not inferred from a failed download.  `--offline`
# performs only local version probes against --prefix and exits with a named
# missing-oracle error; it never reads a tag payload and never invokes the
# Rust/Clang restore helper. An online run may reuse verified local payloads
# or download the exact locked Rust/Clang assets.
# ============================================================================
set -euo pipefail

REPO=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)

PREFIX=${LCCC_ORACLE_PREFIX:-/opt/oracles}
BIN_DIR=/usr/local/bin
WITH_SHIMS=1
WITH_BINUTILS=0
VERIFY_ONLY=0
OFFLINE=0

# The pins. These are the versions the gates name; a tag that carries a
# different version is a provisioning error, not a new default.
GCC_VERSION=16.2.0
CLANG_VERSION=23.1.4
RUST_VERSION=1.99.1
MOLD_VERSION=3.0.0
BINUTILS_VERSION=2.47

while [ $# -gt 0 ]; do
    case $1 in
        --prefix) PREFIX=$2; shift 2 ;;
        --bin-dir) BIN_DIR=$2; shift 2 ;;
        --no-shims) WITH_SHIMS=0; shift ;;
        --with-binutils) WITH_BINUTILS=1; shift ;;
        --verify-only) VERIFY_ONLY=1; shift ;;
        --offline) OFFLINE=1; shift ;;
        -h|--help) sed -n '2,80p' "$0"; exit 0 ;;
        *) echo "oracle_toolchains_from_tags: unknown argument: $1" >&2; exit 2 ;;
    esac
done

log() { printf '[oracles] %s\n' "$*"; }
die() { printf '[oracles] FATAL: %s\n' "$*" >&2; exit 1; }

# ---- tag payload table ------------------------------------------------------
# tag:member:sha256-of-the-member. The digest is repeated here rather than read
# from the tag's MANIFEST at runtime for one reason: a corrupted/rewritten tag
# must not be able to vouch for itself. `manifest_crosscheck` below then proves
# the in-tree MANIFEST and this table agree, so a legitimate re-issue of a tag
# fails loudly here (with both digests printed) instead of silently.
TAG_GCC=harness-cache-1
GCC_MEMBER=gcc-16-bookworm-x86_64.tar.xz
BINUTILS_MEMBER=binutils-2.47.tar.xz

GCC_SHA=4fa6af4f6b29991e31631af18d83964a69caa9daf3f78546d4dc30f39880cd8a
BINUTILS_SHA=154ab23b60070e8f27013c22977f1129425d67d1e8acd6e13010e617811e4cff

cd "$REPO"

# ---- payload transport ------------------------------------------------------
# A member may be committed whole, or split into `<member>.partNN` chunks when
# a host refuses large single blobs. Concatenating the parts in lexical order
# reproduces the member byte for byte; the digest check that follows is what
# proves it, so the split width is not part of any contract here.
materialise() {  # materialise <tag> <member> <dest>
    local tag=$1 member=$2 dest=$3 parts
    if git cat-file -e "$tag:$member" 2>/dev/null; then
        git cat-file blob "$tag:$member" > "$dest"
        return 0
    fi
    parts=$(git ls-tree --name-only "$tag" | grep -E "^$(printf '%s' "$member" | sed 's/[.[\*^$()+?{|]/\\&/g')\.part[0-9]+$" | sort || true)
    [ -n "$parts" ] || return 1
    : > "$dest"
    local p
    for p in $parts; do
        git cat-file blob "$tag:$p" >> "$dest"
    done
}

verify_digest() {  # verify_digest <file> <expected> <label>
    local file=$1 expected=$2 label=$3 actual
    actual=$(sha256sum "$file" | awk '{print $1}')
    if [ "$actual" != "$expected" ]; then
        die "$label digest mismatch: expected $expected, got $actual"
    fi
    log "$label digest OK ($(stat -c %s "$file") bytes)"
}

# The in-tree manifest must agree with the table above; see TAG payload table.
manifest_crosscheck() {  # manifest_crosscheck <tag> <member> <sha>
    local tag=$1 member=$2 expected=$3 from_tag
    from_tag=$(git show "$tag:MANIFEST.sha256" 2>/dev/null | awk -v m="$member" '
        { sub(/^\.\//, "", $2); if ($2 == m || $2 == m ".part00") print $1 }' | head -n 1)
    if [ -z "$from_tag" ]; then
        die "$tag:MANIFEST.sha256 lists no digest for $member"
    fi
    if [ "$from_tag" != "$expected" ]; then
        die "$tag:MANIFEST.sha256 disagrees with this script about $member"$'\n'"  manifest: $from_tag"$'\n'"  script  : $expected"$'\n'"  A re-issued payload needs both updated together."
    fi
}

# ---- install layout ---------------------------------------------------------
GCC_ROOT=$PREFIX/gcc-$GCC_VERSION
CLANG_ROOT=$PREFIX/clang-$CLANG_VERSION
RUST_ROOT=$PREFIX/rust-$RUST_VERSION
gcc_bin=$GCC_ROOT/bin/gcc-16
clang_bin=$CLANG_ROOT/bin/clang-23

probe_version() {  # probe_version <binary> <pattern> <label> ; prints the line
    local bin=$1 pattern=$2 label=$3 out
    [ -x "$bin" ] || die "$label not executable at $bin"
    out=$("$bin" --version 2>&1 | head -n 1)
    case $out in
        *"$pattern"*) printf '%s\n' "$out" ;;
        *) die "$label at $bin reports '$out', expected to contain '$pattern'" ;;
    esac
}

# ============================================================================
# Explicit offline path: probes only the existing prefix.  Do this before
# requiring git, tar, xz, or curl so a disconnected/minimal verification host
# gets the real missing-oracle diagnostic rather than a misleading transport
# prerequisite failure.
# ============================================================================
if [ "$OFFLINE" = 1 ] || [ "$VERIFY_ONLY" = 1 ]; then
    log "verification mode: refusing tag materialisation and toolchain downloads"
    probe_version "$gcc_bin" "GCC) $GCC_VERSION" "gcc-16" >/dev/null
    probe_version "$clang_bin" "version $CLANG_VERSION " "clang-23" >/dev/null
    probe_version "$RUST_ROOT/bin/rustc" "rustc $RUST_VERSION " "rustc" >/dev/null
    probe_version "$PREFIX/bin/mold" "$MOLD_VERSION" "mold" >/dev/null
    log "offline verification: gcc-16 $GCC_VERSION, clang-23 $CLANG_VERSION, Rust $RUST_VERSION, mold $MOLD_VERSION all present"
    exit 0
fi

for tool in git sha256sum tar xz python3 curl; do
    command -v "$tool" >/dev/null 2>&1 || die "required host tool missing: $tool"
done

mkdir -p "$PREFIX" || die "cannot create prefix $PREFIX (use --prefix DIR)"
DL=$PREFIX/dl
mkdir -p "$DL"

# ============================================================================
# Path 2: restore
# ============================================================================
restore_toolchain() {  # restore_toolchain <tag> <member> <sha> <root> <label>
    local tag=$1 member=$2 sha=$3 root=$4 label=$5
    manifest_crosscheck "$tag" "$member" "$sha"
    if [ -d "$root" ]; then
        log "$label already extracted in $root (reused)"
        return 0
    fi
    local file=$DL/$member
    if [ -f "$file" ] && [ "$(sha256sum "$file" | awk '{print $1}')" = "$sha" ]; then
        log "$label payload already verified in $DL (reused)"
    else
        log "materialising $label from tag $tag:$member"
        materialise "$tag" "$member" "$file" || die "tag $tag carries no $member"
        verify_digest "$file" "$sha" "$label"
    fi
    log "extracting $label into $root"
    mkdir -p "$root"
    tar -xf "$file" -C "$root" --strip-components=1 || die "extraction of $label failed"
}

restore_toolchain "$TAG_GCC" "$GCC_MEMBER" "$GCC_SHA" "$GCC_ROOT" "gcc-$GCC_VERSION"
# Restore only the published exact-version payloads after hash verification.
python3 "$REPO/scripts/restore_toolchain_cache.py" --prefix "$PREFIX"

# mold 3.0.0 no longer comes from the tag (the tag binary is 2.42.1):
# the dedicated installer verifies the upstream release digest, refuses
# a foreign/stale prefix, and proves a link.  An old 2.42.1 binary in
# the prefix is therefore never silently reused -- it fails the probe
# below and points at a fresh prefix instead of vouching for itself.
mkdir -p "$PREFIX/bin"
mold_prefix=$PREFIX/mold-$MOLD_VERSION
bash "$REPO/scripts/ci_install_mold_oracle.sh" --prefix "$mold_prefix" \
    || die "mold $MOLD_VERSION provisioning failed"
ln -sfn "$mold_prefix/bin/mold" "$PREFIX/bin/mold"
# The `ld.mold` spelling is what `-fuse-ld=mold` resolves when it looks for a
# link-time backend by that name; keeping both names pointing at one binary
# means a gate can request either without a second copy on disk.
ln -sf mold "$PREFIX/bin/ld.mold"

# binutils sources are verified even when they are not built: the digest is
# part of what the tag promises, and a corrupt payload must not wait for a
# later --with-binutils run to be discovered.
manifest_crosscheck "$TAG_GCC" "$BINUTILS_MEMBER" "$BINUTILS_SHA"
if [ -f "$DL/$BINUTILS_MEMBER" ] || [ "$WITH_BINUTILS" = 1 ]; then
    [ -f "$DL/$BINUTILS_MEMBER" ] || materialise "$TAG_GCC" "$BINUTILS_MEMBER" "$DL/$BINUTILS_MEMBER"
    verify_digest "$DL/$BINUTILS_MEMBER" "$BINUTILS_SHA" "binutils-$BINUTILS_VERSION sources"
fi
if [ "$WITH_BINUTILS" = 1 ]; then
    if [ -x "$PREFIX/binutils-$BINUTILS_VERSION/bin/as" ]; then
        log "binutils $BINUTILS_VERSION already installed (reused)"
    else
        log "installing binutils $BINUTILS_VERSION from the reviewed binary cache"
        bash "$REPO/scripts/ensure_binutils_247_binary.sh" --prefix "$PREFIX/binutils-$BINUTILS_VERSION" \
            || die "binutils $BINUTILS_VERSION binary install failed"
    fi
fi

# ---- shims ------------------------------------------------------------------
# Gates resolve their oracles by name. The shims are plain symlinks so that a
# `--version` through the shim is the tool's own answer, not a wrapper's.
if [ "$WITH_SHIMS" = 1 ]; then
    if [ ! -d "$BIN_DIR" ]; then
        die "shim directory $BIN_DIR does not exist (use --bin-dir)"
    fi
    if [ ! -w "$BIN_DIR" ]; then
        die "shim directory $BIN_DIR is not writable by $(id -un); re-run with sudo or --bin-dir"
    fi
    install_shim() {  # install_shim <name> <target>
        ln -sfn "$2" "$BIN_DIR/$1"
        log "shim $BIN_DIR/$1 -> $2"
    }
    for name in gcc-16 g++-16 cpp-16; do
        install_shim "$name" "$GCC_ROOT/bin/$name"
    done
    for name in clang clang-23; do
        install_shim "$name" "$CLANG_ROOT/bin/$name"
    done
    install_shim clang++ "$CLANG_ROOT/bin/clang-23"
    for name in rustc cargo rustfmt clippy-driver; do
        install_shim "$name" "$RUST_ROOT/bin/$name"
    done
    for name in cargo-clippy cargo-fmt rustdoc; do
        if [ -x "$RUST_ROOT/bin/$name" ]; then
            install_shim "$name" "$RUST_ROOT/bin/$name"
        fi
    done
    install_shim mold "$PREFIX/bin/mold"
    install_shim ld.mold "$PREFIX/bin/mold"
    if [ "$WITH_BINUTILS" = 1 ] && [ -x "$PREFIX/binutils-$BINUTILS_VERSION/bin/as" ]; then
        install_shim gas-2.47 "$PREFIX/binutils-$BINUTILS_VERSION/bin/as"
        install_shim ld.bfd-2.47 "$PREFIX/binutils-$BINUTILS_VERSION/bin/ld"
    fi
fi

# ---- post-conditions --------------------------------------------------------
# A provisioning step that exits 0 without a working oracle is the failure mode
# this whole script exists to prevent, so the exit status is earned by running
# each tool, not by having written a file with the right name.
gcc_line=$(probe_version "$gcc_bin" "GCC) $GCC_VERSION" "gcc-16")
clang_line=$(probe_version "$clang_bin" "version $CLANG_VERSION" "clang-23")
mold_line=$(probe_version "$PREFIX/bin/mold" "$MOLD_VERSION" "mold")
log "$gcc_line"
log "$clang_line"
log "$mold_line"

# The linker the oracles use is the distribution's by default; mold is the
# research preference. Proving the driver can actually LINK (not just print a
# version) catches a prefix whose headers or libgcc were not carried with it.
probe_dir=$(mktemp -d)
trap 'rm -rf "$probe_dir"' EXIT
printf 'int main(void){return 0;}\n' > "$probe_dir/t.c"
"$gcc_bin" -O2 "$probe_dir/t.c" -o "$probe_dir/t16" -B"$GCC_ROOT/bin" 2>/dev/null \
    || die "gcc-16 cannot link a trivial program (incomplete prefix?)"
"$clang_bin" -O2 -fuse-ld=mold -B"$PREFIX/bin" "$probe_dir/t.c" -o "$probe_dir/t23" 2>/dev/null \
    || die "clang-23 cannot link a trivial program with mold (missing crt or headers?)"
log "link probes: gcc-16 OK, clang-23+mold OK"

# An env file keeps the paths available to a non-shim caller (a gate run from
# a different PATH, or a future session script that sources it).
envfile=$PREFIX/env.sh
{
    printf '# Generated by scripts/oracle_toolchains_from_tags.sh — source me.\n'
    printf 'export LCCC_ORACLE_PREFIX=%q\n' "$PREFIX"
    printf 'export GCC=%q\n' "$gcc_bin"
    printf 'export CXX=%q\n' "$GCC_ROOT/bin/g++-16"
    printf 'export CLANG=%q\n' "$clang_bin"
    printf 'export MOLD=%q\n' "$PREFIX/bin/mold"
    printf 'export RUSTC=%q\n' "$RUST_ROOT/bin/rustc"
    printf 'export CARGO=%q\n' "$RUST_ROOT/bin/cargo"
    printf 'export PATH=%q:%q:%q:%q:%q:"$PATH"\n' "$RUST_ROOT/bin" "$CLANG_ROOT/bin" "$GCC_ROOT/bin" "$PREFIX/bin" "$BIN_DIR"
} > "$envfile"
log "env file: $envfile"
log 'done.'
