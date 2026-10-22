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

BUNDLE_DIR=${LCCC_TOOLCHAIN_BUNDLE_DIR:-$HOME/.cache/lccc-toolchain}
ROOT=/
NO_BUNDLE=0
FORCE_INSTALL=0
# `python3-yaml` is a gate dependency, not a compiler dependency, but it is the
# same kind of provisioning: an apt transaction per lane for an immutable
# package.  Folding it in here removes a network call from every lane that used
# to run `apt-get install -y python3-yaml` on its own.
EXTRAS="qemu-user mold lld python3-yaml"

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
                  "qemu-user mold lld python3-yaml"
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

# ── Fast path: the cached bundle ───────────────────────────────────────────
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
        say "toolchain ready from the cached bundle in $(elapsed "$started" "$(now)")"
        exit 0
    fi
    # A corrupt or incomplete archive is a cache miss, not a provisioning
    # failure: fall through to the installer, which then rewrites the bundle.
    say "cached bundle failed verification; installing from packages"
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

started=$(now)
before=""
if (( ! NO_BUNDLE )); then
    mkdir -p "$BUNDLE_DIR"
    before=$BUNDLE_DIR/.packages.before
    snapshot_packages > "$before"
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
        as_root apt-get install -y --no-install-recommends "${extras_after[@]}"
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
    else
        say "no package delta after install; keeping the installer's own state"
        NO_BUNDLE=1
    fi
    rm -f "$before" "$after"
fi

say "toolchain provisioned in $(elapsed "$started" "$(now)")"

# ── Contract probes ────────────────────────────────────────────────────────
# One owner of "what a provisioned lane must be able to do": the same probes
# the fast path runs.  This is what makes a bundle-less install (--no-bundle or
# --force-install) as trustworthy as a restored one, and it is why the install
# path cannot silently degrade to a lane without a working i686 multilib.
bash "$BUNDLE_TOOL" verify --bundle "$BUNDLE_DIR" --root "$ROOT"
