#!/usr/bin/env bash
# Provision the hosted-runner C oracles once, then hand every lane the same
# bytes instead of paying the same dpkg transaction seven times per pipeline.
#
# WHY THIS EXISTS
# ---------------
# Six CI jobs independently ran `apt-get install` for the same GCC 16 + i686
# multilib + linker-oracle package set.  Measured on run 39686387347
# (2026-10-21, main @ e623708), that step cost 80-205 s in each job: ~937 s of
# runner time for one transaction, and it sat at the head of the critical path
# because the build lane cannot start compiling until it finishes.
#
# The package set is immutable for a given runner image and installer, so it
# is materialised ONCE into a content-addressed tarball:
#
#   export   collect the files owned by a set of dpkg packages into
#            toolchain.tar.gz + a sha256 + the package/file manifests
#   restore  verify that sha256 and unpack the archive
#   verify   probe the restored tools (version, i686 link, linker oracles)
#
# Measured locally on a 262 MB, 20k-file GCC tree (2026-10-21, 2-core sandbox):
# `tar -czf` 12.0 s, `tar -xzf` 1.9 s, so a restore is a few seconds where the
# hosted apt transaction it replaces cost 80-205 s.
#
# INVARIANTS
# ----------
# * The archive is never trusted because of its filename: `restore` compares
#   `sha256sum -c` against the manifest written by `export`, and the caller
#   (`ci_provision_toolchain.sh`) re-probes the tools afterwards.  A truncated
#   or stale cache therefore falls back to a real install instead of shipping
#   a broken compiler.
# * `export` derives its file list from `dpkg -L` of exactly the packages the
#   caller names, so the bundle can never contain a hand-curated subset that
#   silently loses a header, a crt object or a shared library.
# * Paths are stored relative to `--root` and unpacked with the same layout, so
#   the same code restores into `/` on a runner and into a temporary root in
#   the regression test (which is why the test can exercise tar, hashing and
#   the probe contract without root or a package manager).
# * Documentation, translations and man pages are excluded: no compiler input
#   resolves there, and they are the bulk of the payload.
set -euo pipefail

usage() {
    cat >&2 <<'EOF'
usage: ci_toolchain_bundle.sh export  --bundle DIR --packages "pkg ..." [--root R]
       ci_toolchain_bundle.sh export  --bundle DIR --from-file LIST  [--root R]
       ci_toolchain_bundle.sh restore --bundle DIR [--root R] [--no-verify]
       ci_toolchain_bundle.sh verify  --bundle DIR [--root R]

  export   tar the files owned by --packages (dpkg -L) or listed in
           --from-file, and write toolchain.tar.gz + toolchain.sha256 +
           toolchain.packages + toolchain.files into --bundle.
  restore  verify the archive against its manifest and unpack it under --root.
  verify   check that the tools a CI lane needs exist (and, when --root is /,
           that they run), then print a one-line digest.

  --root defaults to / and exists so the contract test can exercise the whole
  round trip unprivileged instead of mocking tar.
EOF
}

die() {
    echo "ci_toolchain_bundle: $*" >&2
    exit 1
}

say() {
    printf 'ci_toolchain_bundle: %s\n' "$*"
}

# Diagnostics that mean "this bundle is unusable" belong on stderr: a caller
# that only checks the exit status still gets them in the job log, and a
# silent stdout is not mistaken for a successful restore.
problem() {
    printf 'ci_toolchain_bundle: %s\n' "$*" >&2
}

COMMAND=${1:-}
[[ -n $COMMAND ]] || { usage; exit 2; }
shift

BUNDLE=""
ROOT=/
FROM_FILE=""
PACKAGES=""
DO_VERIFY=1
while (( $# )); do
    case $1 in
        --bundle) BUNDLE=${2:-}; shift 2 ;;
        --root) ROOT=${2:-}; shift 2 ;;
        --from-file) FROM_FILE=${2:-}; shift 2 ;;
        --packages) PACKAGES=${2:-}; shift 2 ;;
        --no-verify) DO_VERIFY=0; shift ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown argument: $1" ;;
    esac
done
[[ -n $BUNDLE ]] || { usage; exit 2; }
ROOT=${ROOT%/}
[[ $ROOT == "" ]] && ROOT=/
BUNDLE=${BUNDLE%/}
ARCHIVE=$BUNDLE/toolchain.tar.gz
MANIFEST=$BUNDLE/toolchain.sha256
PACKAGE_MANIFEST=$BUNDLE/toolchain.packages
FILE_MANIFEST=$BUNDLE/toolchain.files

# Resolve a stored (root-relative) path against the root being written/read.
under_root() {
    printf '%s/%s' "${ROOT%/}" "$1"
}

# Every path the archive may contain, as a root-relative name.  dpkg -L yields
# absolute paths; the caller's --from-file may yield either.  Documentation
# trees are dropped here (the only place a path is filtered) so the manifest
# and the archive can never disagree about what a bundle holds.
file_list() {
    local candidate
    while IFS= read -r candidate; do
        [[ -n $candidate ]] || continue
        candidate=${candidate#/}
        case $candidate in
            usr/share/doc/*|usr/share/man/*|usr/share/info/*|usr/share/locale/*) continue ;;
            var/lib/dpkg/*|var/cache/*) continue ;;
        esac
        [[ -e $(under_root "$candidate") || -L $(under_root "$candidate") ]] || continue
        printf '%s\n' "$candidate"
    done | LC_ALL=C sort -u
}

package_manifest() {
    local package version
    for package in $PACKAGES; do
        version=$(dpkg-query -W -f='${Version}' "$package" 2>/dev/null) || version=unknown
        printf '%s %s\n' "$package" "$version"
    done
}

do_export() {
    [[ -d $ROOT ]] || die "--root is not a directory: $ROOT"
    mkdir -p "$BUNDLE"
    local list=$BUNDLE/.files.tmp
    if [[ -n $FROM_FILE ]]; then
        [[ -r $FROM_FILE ]] || die "--from-file is not readable: $FROM_FILE"
        file_list < "$FROM_FILE" > "$list"
    elif [[ -n $PACKAGES ]]; then
        local package
        for package in $PACKAGES; do
            dpkg -L "$package" 2>/dev/null || die "dpkg -L $package failed"
        done | file_list > "$list"
    else
        die "export needs --packages or --from-file"
    fi
    local count
    count=$(wc -l < "$list")
    (( count > 0 )) || die "no files selected for the bundle"
    # --no-recursion stops a directory entry from dragging its subtree in a
    # second time; dpkg -L already lists every owned path.
    tar -czf "$ARCHIVE" --numeric-owner --no-recursion -C "$ROOT" -T "$list"
    ( cd "$BUNDLE" && sha256sum "$(basename "$ARCHIVE")" > "$(basename "$MANIFEST")" )
    cp "$list" "$FILE_MANIFEST"
    if [[ -n $PACKAGES ]]; then package_manifest > "$PACKAGE_MANIFEST"; fi
    rm -f "$list"
    say "exported $count file(s), $(du -h "$ARCHIVE" | cut -f1) -> $ARCHIVE"
}

do_restore() {
    [[ -r $ARCHIVE ]] || die "no bundle archive at $ARCHIVE"
    [[ -d $ROOT ]] || mkdir -p "$ROOT"
    [[ -r $MANIFEST ]] || die "no bundle manifest at $MANIFEST"
    ( cd "$BUNDLE" && sha256sum -c --status "$(basename "$MANIFEST")" ) \
        || die "bundle archive does not match its manifest"
    # --no-same-owner when unprivileged: the contract test restores into a
    # temporary root, where chown(2) to the archive's uids would fail.
    local -a owner=()
    if (( EUID == 0 )); then owner=(--same-owner); fi
    tar -xzpf "$ARCHIVE" -C "$ROOT" "${owner[@]}"
    # Shared libraries that landed under /usr/lib32 (or any multiarch dir) are
    # invisible to the dynamic loader until its cache is rebuilt.
    if [[ $ROOT == / ]] && command -v ldconfig >/dev/null 2>&1; then
        ldconfig 2>/dev/null || true
    fi
    local files=unknown
    [[ -r $FILE_MANIFEST ]] && files=$(wc -l < "$FILE_MANIFEST")
    say "restored $files file(s) into $ROOT"
}

# Tools every consumer lane needs.  `qemu-i386` is how i686 regression rows and
# the linker suite execute 32-bit binaries on a kernel that refuses them.
REQUIRED_BINARIES=(
    usr/bin/gcc-16
    usr/bin/g++-16
    usr/bin/ld.lld
    usr/bin/qemu-i386
    # Memcheck is a contract, not a convenience: the FP/hexadecimal oracles in
    # the contract lanes run their probe under valgrind and REFUSE to run when
    # it is missing (a skipped cross-check would report a comparison that never
    # happened).  A bundle that dropped it would restore a lane whose gates
    # cannot pass, so it is part of what `verify` proves.
    usr/bin/valgrind
)
# The CI gate parsers import PyYAML. Its Debian/Ubuntu package is pure Python
# data, so it needs a path check for prefix restores and an import probe on /.
REQUIRED_FILES=(usr/lib/python3/dist-packages/yaml/__init__.py)
# mold is either /usr/bin/mold or the /usr/bin/ld.mold alias, depending on how
# the distribution split the package.
MOLD_CANDIDATES=(usr/bin/mold usr/bin/ld.mold)

do_verify() {
    local missing=() path
    for path in "${REQUIRED_BINARIES[@]}"; do
        [[ -x $(under_root "$path") ]] || missing+=("/$path")
    done
    for path in "${REQUIRED_FILES[@]}"; do
        [[ -f $(under_root "$path") ]] || missing+=("/$path")
    done
    local mold_found=""
    for path in "${MOLD_CANDIDATES[@]}"; do
        if [[ -x $(under_root "$path") ]]; then mold_found=$path; break; fi
    done
    [[ -n $mold_found ]] || missing+=("/usr/bin/mold (or /usr/bin/ld.mold)")
    if (( ${#missing[@]} )); then
        problem "missing from $ROOT: ${missing[*]}"
        return 1
    fi
    # Probing only makes sense against a live root: the regression test's
    # fixture binaries are shell scripts, and executing them would test the
    # fixture instead of the bundle contract.
    if [[ $ROOT == / ]]; then
        local version probe
        version=$(gcc-16 -dumpfullversion) || { problem "gcc-16 does not run"; return 1; }
        [[ ${version%%.*} == 16 ]] || { problem "gcc-16 is $version, not 16.x"; return 1; }
        probe=$(mktemp -d)
        if ! printf 'int main(void) { return 0; }\n' \
                | gcc-16 -m32 -x c - -o "$probe/m32" >/dev/null 2>&1; then
            rm -rf "$probe"
            problem "gcc-16 -m32 cannot link: the i686 multilib is incomplete"
            return 1
        fi
        mold --version >/dev/null 2>&1 || { rm -rf "$probe"; problem "mold does not run"; return 1; }
        # `mold --version` is not the contract the BUILD needs: gcc/clang
        # resolve `-fuse-ld=mold` through an `ld.mold` sibling, and a mold
        # without it passes the version probe and then fails every link with
        # "collect2: fatal error: cannot find 'ld'".  Probe the real pairing.
        if ! printf 'int main(void) { return 0; }\n' \
                | gcc-16 -fuse-ld=mold -x c - -o "$probe/mold-link" >/dev/null 2>&1; then
            rm -rf "$probe"
            problem "gcc-16 cannot link through mold: \`-fuse-ld=mold\` needs the \`ld.mold\` symlink beside \`mold\`"
            return 1
        fi
        ld.lld --version >/dev/null 2>&1 || { rm -rf "$probe"; problem "ld.lld does not run"; return 1; }
        qemu-i386 --version >/dev/null 2>&1 || { rm -rf "$probe"; problem "qemu-i386 does not run"; return 1; }
        # Presence is not enough: valgrind aborts at startup when the loader's
        # debug symbols are missing, which is the failure a bundle without
        # libc6-dbg would restore.  `--version` alone does not touch them, so
        # run one real Memcheck pass over a trivial program.
        if ! printf 'int main(void) { return 0; }\n' \
                | gcc-16 -x c - -o "$probe/vg" >/dev/null 2>&1 \
                || ! valgrind --error-exitcode=99 --leak-check=full -q "$probe/vg" >/dev/null 2>&1; then
            rm -rf "$probe"
            problem "valgrind cannot run Memcheck (missing libc6-dbg for the loader, or a broken build)"
            return 1
        fi
        rm -rf "$probe"
        python3 -c 'import yaml' >/dev/null 2>&1 || {
            problem "python3-yaml cannot be imported by python3"; return 1;
        }
        say "verified live root: gcc-16 $version (mold + Memcheck probes), $(valgrind --version), python3-yaml"
    else
        say "verified layout under $ROOT (probes need a live root)"
    fi
}

case $COMMAND in
    export) do_export ;;
    restore) do_restore; if (( DO_VERIFY )); then do_verify; fi ;;
    verify) do_verify ;;
    *) usage; exit 2 ;;
esac
