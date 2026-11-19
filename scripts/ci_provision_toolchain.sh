#!/usr/bin/env bash
# Give a CI lane its C oracles in seconds instead of an apt transaction.
#
# WHY THIS EXISTS
# ---------------
# Run 39686387347 (2026-10-21, main @ e623708) spent 80-205 s installing the
# same GCC 16 + i686 multilib + qemu + mold + lld package set in six jobs, and
# another 125 s in the benchmark lane: ~1062 s of runner time for one dpkg
# transaction, every second of it at the head of a job's critical path.  The
# package set is immutable for a given runner image and installer pair, so it
# is materialised once into a content-addressed tarball and restored from the
# Actions cache afterwards.
#
# Two provisioning paths, in the order they are tried:
#
#   1. cached bundle    `ci_toolchain_bundle.sh restore` of a tarball built
#                       once per (runner image, installer) pair.  Measured
#                       restore: a few seconds for a 262 MB tree.
#   2. apt              `ci_install_gcc16.sh` (which already falls back to a
#                       local .deb archive when the package index is
#                       unavailable), then the extras in one transaction.
#
# WHICH BUNDLE MAY BE RESTORED
# ----------------------------
# A bundle is the files of one apt transaction, unpacked over `/`: a delta that
# is valid on exactly the package baseline it was computed against.  Every
# bundle exported here is therefore stamped with `ci_toolchain_key.sh` -- a
# digest of that baseline and of the installer -- and (1) is taken only when the
# stamp is this runner's.  The workflows put the same string in the cache key,
# so a new runner image is a cache miss instead of an exact hit on another
# image's files.  (It used to be an exact hit; `ci_toolchain_key.sh` records
# what that cost in run 44345167051.)
#
# WHAT A NEW IMAGE STARTS FROM
# ----------------------------
# The package files that built a bundle are kept beside it (`debs/`, with apt's
# index), and the workflows restore the newest entry of ANY image when the
# exact key misses.  Such a bundle is never unpacked -- its stamp is not ours --
# but its archive is handed to apt, which decides package by package what is
# still valid for this image and fetches only the rest.  Measured on the image
# change above, that rest is the five glibc-versioned packages (16 MB from the
# Ubuntu mirror) instead of the 319 MB GCC set from the Launchpad PPA.  The
# rebuilt bundle is stamped, and because the exact key missed, the cache action
# saves it: the next job on this image restores in seconds.
#
# Every lane provisions the SAME package set, i686 multilib included: a bundle
# that cannot link `-m32` fails `verify`, so a 64-bit-only variant would poison
# the shared cache entry for the lanes that need the 32-bit oracles.
#
# The bundle is written on a miss, so the next run starts at (1).  This is a
# cache, not a source of truth: the archive is verified against the sha256 that
# `export` wrote, and then the *tools themselves* are probed.  A failed probe
# discards the bundle and installs, so a stale or truncated cache can never
# turn into a green lane running the wrong compiler.
#
# `--root` provisions a prefix instead of / (the regression test uses it to
# exercise the fast path unprivileged; the live-root probes are skipped there).
# The restore is escalated because unpacking into / and running `ldconfig` are
# root operations; the install path is not, because `ci_install_gcc16.sh` and
# apt already escalate for themselves.
set -euo pipefail

REPO=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
BUNDLE_TOOL=$REPO/scripts/ci_toolchain_bundle.sh
KEY_TOOL=$REPO/scripts/ci_toolchain_key.sh

BUNDLE_DIR=${LCCC_TOOLCHAIN_BUNDLE_DIR:-$HOME/.cache/lccc-toolchain}
# How this lane got its toolchain, for scripts/ci_verify_oracles.sh to report.
# "Which compiler did this run actually use, and did it come from the bundle or
# from apt?" is unanswerable from the job log through the check-run API, and it
# is the first question asked of a lane whose oracle gate failed.
PROVENANCE=${LCCC_PROVENANCE_FILE:-$HOME/.cache/lccc-toolchain/last-provision.json}
ROOT=/
NO_BUNDLE=0
FORCE_INSTALL=0
# `python3-yaml` is a gate dependency, not a compiler dependency, but it is the
# same kind of provisioning: an apt transaction per lane for an immutable
# package.  Folding it in here removes a network call from every lane that used
# to run `apt-get install -y python3-yaml` on its own.
#
# `valgrind` is a GATE dependency too, and it was the one missing: the FMA
# contract gate's hexadecimal/FP oracles run the compiled probe under Memcheck
# (scripts/check_hex_float_oracles.py, check_signed_mask_oracles.py), which
# requires the tool -- and refuses to run without it rather than skipping the
# check.  No lane installed it, so the contract that needed it could only pass
# where the tool happened to be present.  `libc6-dbg` is part of the same
# contract, not a debugging nicety: Memcheck redirects `strlen` and friends in
# `/lib64/ld-linux-x86-64.so.2`, and the distribution ships that loader
# stripped, so valgrind aborts at startup ("a function redirection which is
# mandatory for this platform-tool combination cannot be set up") unless the
# matching debug symbols are installed.  Both are immutable for a runner
# image, so they belong in the same cached bundle as the compilers.
EXTRAS="qemu-user mold lld python3-yaml valgrind libc6-dbg gdb"

usage() {
    cat >&2 <<'EOF'
usage: ci_provision_toolchain.sh [--bundle-dir DIR] [--root DIR]
                                 [--extras "pkg ..."] [--no-bundle]
                                 [--force-install]

  --multilib      accepted and implied: the i686 multilib is part of the
                  contract, and a bundle that cannot link -m32 is refused by
                  `verify`, so every lane shares ONE cache entry
  --bundle-dir    where the content-addressed toolchain bundle lives
  --root DIR      restore into DIR instead of / (bundle path only)
  --extras PKGS   oracle packages installed with the compiler, default:
                  "qemu-user mold lld python3-yaml valgrind libc6-dbg gdb"
  --no-bundle     never read or write the bundle (pure installer path)
  --force-install ignore a cached bundle and reinstall from packages
EOF
}

die() {
    printf 'ci_provision_toolchain: %s\n' "$*" >&2
    exit 2
}

say() {
    printf 'ci_provision_toolchain: %s\n' "$*"
}

# A check-run annotation: the job log is not reachable through the Checks API,
# and "this lane did not use its bundle" was invisible there for as long as it
# only cost time.  Silent outside Actions.
annotate() {
    [[ ${GITHUB_ACTIONS:-} == true ]] || return 0
    local level=$1 title=$2 message=$3
    message=${message//%/%25}
    message=${message//$'\r'/%0D}
    message=${message//$'\n'/%0A}
    printf '::%s title=%s::%s\n' "$level" "$title" "$message"
}

# Best effort by construction: a lane must never fail because its provenance
# note could not be written.
record_provenance() {
    local source=$1 seconds=$2
    mkdir -p -- "$(dirname -- "$PROVENANCE")" 2>/dev/null || return 0
    printf '{"source": "%s", "toolchain_seconds": "%s", "at": "%s"}\n' \
        "$source" "$seconds" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" >"$PROVENANCE" 2>/dev/null || true
}

while (( $# )); do
    case $1 in
        --multilib) shift ;;
        --bundle-dir) BUNDLE_DIR=${2:?--bundle-dir needs a value}; shift 2 ;;
        --root) ROOT=${2:?--root needs a value}; shift 2 ;;
        # `${2?}` (not `${2:?}`): an explicitly EMPTY --extras is the documented
        # way to say "no extra oracles", while a missing value is an error.
        --extras) EXTRAS=${2?--extras needs a value (use "" for none)}; shift 2 ;;
        --no-bundle) NO_BUNDLE=1; shift ;;
        --force-install) FORCE_INSTALL=1; shift ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown argument: $1" ;;
    esac
done

ROOT=${ROOT%/}
[[ $ROOT == "" ]] && ROOT=/

if (( EUID == 0 )); then
    as_root() { "$@"; }
else
    as_root() { sudo -- "$@"; }
fi

now() { date +%s%N; }
elapsed() {
    python3 -c "import sys; print(f'{(int(sys.argv[2])-int(sys.argv[1]))/1e9:.1f}s')" "$1" "$2" 2>/dev/null || echo "?"
}

# The bundle's identity, taken before this script changes the package baseline
# it describes.
BUNDLE_KEY=""
STAMP=$BUNDLE_DIR/toolchain.key
ARCHIVE_DIR=$BUNDLE_DIR/debs
if (( ! NO_BUNDLE )); then
    BUNDLE_KEY=$(bash "$KEY_TOOL") || die "cannot compute the toolchain bundle key"
    [[ -n $BUNDLE_KEY ]] || die "the toolchain bundle key is empty"
fi

# An archive that is not going to be restored must not survive into the cache
# entry this job saves: under this runner's key it would be an exact hit on
# files that are not this runner's.  Its package archive stays.
discard_bundle_archive() {
    rm -f -- "$BUNDLE_DIR/toolchain.tar.gz" "$BUNDLE_DIR/toolchain.sha256" \
        "$BUNDLE_DIR/toolchain.packages" "$BUNDLE_DIR/toolchain.files" "$STAMP"
}

# ── Fast path: the cached bundle ───────────────────────────────────────────
if (( ! NO_BUNDLE && ! FORCE_INSTALL )) && [[ -r $BUNDLE_DIR/toolchain.tar.gz ]]; then
    stamp=""
    if [[ -r $STAMP ]]; then stamp=$(<"$STAMP"); fi
    if [[ $stamp != "$BUNDLE_KEY" ]]; then
        # Another image's (or another installer's) delta.  Unpacking it is how
        # a lane ends up with debug symbols for a loader it is not running, and
        # with tools dpkg does not know about -- which the installer below
        # would then skip, and the re-exported bundle would lack.
        say "cached bundle is for ${stamp:-an unstamped baseline}, this runner is $BUNDLE_KEY; installing from packages"
        annotate notice "Toolchain bundle rebuilt for this runner image" \
            "The cached bundle was built for ${stamp:-an unstamped baseline}; this runner is $BUNDLE_KEY. Reinstalling from the package archive it carries."
        discard_bundle_archive
    fi
fi
if (( ! NO_BUNDLE && ! FORCE_INSTALL )) && [[ -r $BUNDLE_DIR/toolchain.tar.gz ]]; then
    started=$(now)
    # `restore` verifies the archive's sha256 and probes the tools, so a
    # successful return means the lane has a working gcc-16 -m32, mold, ld.lld
    # and qemu-i386 -- the exact contracts the old apt step established.
    #
    # Escalate ONLY for the live root: unpacking into / and running ldconfig
    # are privileged, while a `--root PREFIX` restore is by construction
    # unprivileged (which is what keeps the contract test sudo-free).
    restore=(bash "$BUNDLE_TOOL" restore --bundle "$BUNDLE_DIR" --root "$ROOT")
    if [[ $ROOT == / ]]; then
        if as_root "${restore[@]}"; then ok=1; else ok=0; fi
    else
        if "${restore[@]}"; then ok=1; else ok=0; fi
    fi
    if (( ok )); then
        took=$(elapsed "$started" "$(now)")
        say "toolchain ready from the cached bundle in $took"
        record_provenance bundle "$took"
        exit 0
    fi
    # A corrupt or incomplete archive is a cache miss, not a provisioning
    # failure: fall through to the installer, which then rewrites the bundle.
    say "cached bundle failed verification; installing from packages"
    # Unlike a bundle for another image this one claimed to be ours, and its
    # cache entry cannot be replaced (the key exists): every lane on this
    # image will pay the install until the key changes.  Say so where it shows.
    annotate warning "Cached toolchain bundle rejected" \
        "The bundle stamped $BUNDLE_KEY failed verification on the image it was built for; installing from packages. Its cache entry is immutable, so this repeats in every lane until the entry is deleted."
    discard_bundle_archive
fi

# ── Install path ───────────────────────────────────────────────────────────
# Installing into a prefix is not a supported operation: apt and dpkg write to
# the live root, so `--root` only makes sense for the cached-bundle fast path
# (and the regression test that exercises it).
if [[ $ROOT != / ]]; then
    die "cannot install into $ROOT; --root is only supported for a restored bundle"
fi

# Package delta, so the exported bundle holds exactly the transaction's files
# (the runner image already ships libc, binutils and friends).
snapshot_packages() {
    dpkg-query -W -f='${Package} ${Version}\n' 2>/dev/null | LC_ALL=C sort
}

# Adopt the package files of a second archive.  The lanes that can start from
# an empty bundle namespace keep a weekly `lccc-apt-debs` cache under its own
# key, so a change of the bundle key format does not become a 319 MB download.
seed_archive_from() {
    local source=$1 file target
    [[ -n $source && -d $source && ! $source -ef $ARCHIVE_DIR ]] || return 0
    mkdir -p "$ARCHIVE_DIR/lists"
    for file in "$source"/*.deb "$source"/lists/*; do
        [[ -f $file ]] || continue
        target=$ARCHIVE_DIR/${file#"$source"/}
        # Same name, same package file: apt names them <package>_<version>_<arch>.
        # Best effort: a file that could not be adopted is one apt fetches.
        [[ -e $target ]] || ln -- "$file" "$target" 2>/dev/null \
            || cp -p -- "$file" "$target" || true
    done
}

# Keep exactly the package files this installation consists of.  A seeded
# archive also holds the previous image's versions, and without this every
# rebuilt bundle would inherit all of them: one PPA rebuild of GCC is 319 MB.
# Decided from dpkg's own state, never from the index -- a PPA index that
# failed to download would make every GCC package look obsolete.
prune_archive() {
    local -A installed=()
    local package arch version file name rest
    while IFS=$'\t' read -r package arch version; do
        if [[ -n $package && -n $arch ]]; then
            installed["${package}_${arch}"]=$version
        fi
    done < <(dpkg-query -W -f='${Package}\t${Architecture}\t${Version}\n' 2>/dev/null || true)
    # An unreadable package database is not evidence that nothing is installed.
    (( ${#installed[@]} )) || return 0
    for file in "$ARCHIVE_DIR"/*.deb; do
        [[ -f $file ]] || continue
        name=${file##*/}
        name=${name%.deb}
        package=${name%%_*}
        rest=${name#*_}
        arch=${rest##*_}
        version=${rest%_*}
        # apt writes an epoch's colon as %3a in the file name.
        version=${version//%3a/:}
        version=${version//%3A/:}
        [[ ${installed["${package}_${arch}"]:-} == "$version" ]] || rm -f -- "$file"
    done
    rm -rf -- "$ARCHIVE_DIR/partial" "$ARCHIVE_DIR/lists/partial"
}

started=$(now)
before=""
if (( ! NO_BUNDLE )); then
    mkdir -p "$BUNDLE_DIR"
    before=$BUNDLE_DIR/.packages.before
    snapshot_packages > "$before"
    # The archive belongs to the bundle: whatever built this tree is what the
    # next image starts from.
    seed_archive_from "${LCCC_CI_APT_ARCHIVE_DIR:-}"
    export LCCC_CI_APT_ARCHIVE_DIR=$ARCHIVE_DIR
fi
# The same archive and index the two installers below use.  The extras were
# the one transaction that bypassed them: 36.6 MB from the mirror on every
# install, at 87 kB/s in the run that timed out, and resolved against the
# image's own (older) index instead of the one just refreshed.
archive_opts=()
if [[ -n ${LCCC_CI_APT_ARCHIVE_DIR:-} ]]; then
    mkdir -p "$LCCC_CI_APT_ARCHIVE_DIR/partial" "$LCCC_CI_APT_ARCHIVE_DIR/lists/partial"
    archive_opts=(
        -o "Dir::Cache::archives=$LCCC_CI_APT_ARCHIVE_DIR"
        -o APT::Keep-Downloaded-Packages=true
        -o "Dir::State::lists=$LCCC_CI_APT_ARCHIVE_DIR/lists"
    )
fi

bash "$REPO/scripts/ci_install_gcc16.sh" --multilib
if [[ -n $EXTRAS ]]; then
    # The differential *linker* oracles have their own installer, and it is not
    # interchangeable with a bare apt line: it retries a transient mirror
    # failure, keys resolution on `apt-cache pkgnames` rather than on apt's
    # exit status, fails closed unless both binaries are on PATH, and costs
    # nothing on a warm runner. This lane is the only place that installs
    # packages for every job, so routing the pair through it is what keeps that
    # discipline in force now that the six per-job copies are gone.
    #
    # The delegation is all-or-nothing on purpose: the installer's contract is
    # "exit 0 means mold AND ld.lld are on PATH" -- the exact predicate
    # tests/linker/run_linker_tests.py uses. A caller that asks for only one of
    # them does not want that contract, so a partial request stays on apt.
    extras_after=()
    want_mold=0
    want_lld=0
    for pkg in $EXTRAS; do
        case $pkg in
            mold) want_mold=1 ;;
            lld) want_lld=1 ;;
            *) extras_after+=("$pkg") ;;
        esac
    done
    if (( want_mold && want_lld )); then
        bash "$REPO/scripts/ci_install_link_oracles.sh"
    else
        (( want_mold )) && extras_after+=(mold)
        (( want_lld )) && extras_after+=(lld)
    fi
    if (( ${#extras_after[@]} )); then
        extras_ok=0
        for attempt in 1 2 3; do
            if (( attempt > 1 )); then
                say "retrying extras install (attempt $attempt/3) after refreshing apt index"
                as_root apt-get update -qq "${archive_opts[@]}" || true
            fi
            if as_root apt-get install -y --no-install-recommends \
                    "${archive_opts[@]}" "${extras_after[@]}"; then
                extras_ok=1
                break
            fi
        done
        (( extras_ok )) || die "failed to install extra oracle packages after 3 attempts: ${extras_after[*]}"
    fi
fi

if (( ! NO_BUNDLE )); then
    after=$BUNDLE_DIR/.packages.after
    snapshot_packages > "$after"
    # Every package the transaction added OR changed version.  An upgrade of a
    # preinstalled package (libc6 is the usual one) must be in the bundle for
    # the restored tree to match what the installer just produced.
    mapfile -t delta < <(LC_ALL=C comm -13 "$before" "$after" | awk '{print $1}')
    if (( ${#delta[@]} )); then
        bash "$BUNDLE_TOOL" export --bundle "$BUNDLE_DIR" --packages "${delta[*]}"
        # apt wrote its downloads as root; the cache upload runs unprivileged.
        if (( EUID != 0 )); then
            as_root chown -R "$(id -un):$(id -gn)" "$ARCHIVE_DIR" || true
        fi
        prune_archive
        # Last, so that a bundle is stamped only once it is complete.
        printf '%s\n' "$BUNDLE_KEY" > "$STAMP"
    else
        say "no package delta after install; keeping the installer's own state"
        NO_BUNDLE=1
    fi
    rm -f "$before" "$after"
fi

took=$(elapsed "$started" "$(now)")
say "toolchain provisioned in $took"

# ── Contract probes ────────────────────────────────────────────────────────
# One owner of "what a provisioned lane must be able to do": the same probes
# the fast path runs.  This is what makes a bundle-less install (--no-bundle or
# --force-install) as trustworthy as a restored one, and it is why the install
# path cannot silently degrade to a lane without a working i686 multilib.
bash "$BUNDLE_TOOL" verify --bundle "$BUNDLE_DIR" --root "$ROOT"
record_provenance packages "$took"
