#!/usr/bin/env bash
# Install the bookworm-compatible binutils 2.47 program set from the reviewed
# GitHub-only agent cache. This replaces the per-run source build in linker
# oracle setup, which used to cost ~10-15 minutes on a two-core runner.
set -Eeuo pipefail

REPO=$(CDPATH= cd -- "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
LOCK="$REPO/engineering/toolchains/agent-cache-lock.json"
RESTORE="$REPO/scripts/restore_agent_toolchain_cache.py"

tag=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["tag"])' "$LOCK")
prefix=${LCCC_BINUTILS_PREFIX:-$HOME/.cache/lccc/binutils-2.47}
remote=origin
print_bin=0
verify_only=0
self_test=0
while [ $# -gt 0 ]; do
    case $1 in
        --prefix) prefix=$2; shift 2 ;;
        --tag) tag=$2; shift 2 ;;
        --remote) remote=$2; shift 2 ;;
        --print-bin) print_bin=1; shift ;;
        --verify-only) verify_only=1; shift ;;
        --self-test) self_test=1; shift ;;
        -h|--help) sed -n '2,18p' "$0"; exit 0 ;;
        *) echo "ensure_binutils_247_binary: unknown argument: $1" >&2; exit 2 ;;
    esac
done
[[ $prefix = /* && $prefix != / ]] || { echo "ensure_binutils_247_binary: prefix must be an absolute private directory" >&2; exit 2; }

if [ "$self_test" = 1 ]; then
    python3 "$RESTORE" --self-test
    exit 0
fi
if [ "$verify_only" = 1 ]; then
    # The repository's one version matcher: a whole release token on the
    # identity line.  A bare `grep 2\.47` also accepted 2.47.50.* (a Debian
    # snapshot of binutils master, i.e. 2.48 development), 2.470 and 2.47.1.
    # shellcheck source=../tools/linker/oracle_versions.sh
    . "$REPO/tools/linker/oracle_versions.sh"
    for tool in as ld ld.bfd objdump readelf; do
        [ -x "$prefix/bin/$tool" ] || { echo "missing $prefix/bin/$tool" >&2; exit 1; }
        oracle_require_version "$prefix/bin/$tool" 2.47 || exit 1
    done
    echo "binutils 2.47 binary oracle OK: $prefix/bin"
    exit 0
fi

python3 "$RESTORE" --tag "$tag" --prefix "$prefix" --repo "$REPO" --remote "$remote" --only binutils
bash "$0" --prefix "$prefix" --verify-only
if [ "$print_bin" = 1 ]; then
    echo "$prefix/bin"
fi
