#!/usr/bin/env bash
# Measure the 32 KiB boot-gate .text size for a given lccc commit.
# Usage: bisect_boot.sh <git-rev>
set -uo pipefail
REV=$1
WT=${BISECT_WORKTREE:-/home/user/bisect}
# The LIVE checkout, resolved from this script's own location: the bisection
# contract below (toolchain selection, warning policy) must come from it, not
# from the historical worktree it is about to create.
REPO=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
K=${KERNEL_DIR:-/home/user/kernel-work/linux-6.18.52}
export PATH="$HOME/.cargo/bin:$PATH"
# Resolve from the live checkout, not from a historical worktree: a bisection
# must use the current supported compiler toolchain for every candidate.
# shellcheck source=rust_toolchain.sh
source "$REPO/scripts/rust_toolchain.sh"
lccc_select_rust_toolchain "$REPO"
export CARGO_BUILD_JOBS=2
# A warning-free compiler build is part of the bisection contract as well.
# Permit an explicit local escape hatch only when deliberately bisecting an
# historical revision which predates the warning policy.
if [[ "${LCCC_ALLOW_WARNINGS:-0}" == "1" ]]; then
  export RUSTFLAGS="${RUSTFLAGS:-}"
else
  export RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }-D warnings"
fi

if [[ -d $WT ]]; then
  git -C "$REPO" worktree remove --force "$WT" >/dev/null 2>&1
  rm -rf "$WT"
fi
git -C "$REPO" worktree add -q --detach "$WT" "$REV" >/dev/null 2>&1 || { echo "worktree add failed for $REV"; exit 1; }
cd "$WT"
if ! cargo build --profile fastbuild --locked -j2 >/tmp/bisect-build.log 2>&1; then
  echo "BUILD-FAILED"; tail -5 /tmp/bisect-build.log; exit 1
fi
LCCC=$WT/target/fastbuild/lccc LCCC_LD=$WT/target/fastbuild/lccc-ld \
  KERNEL_DIR=$K OUT=/tmp/bisect-boot bash "$WT/scripts/build_kernel_boot.sh" >/tmp/bisect-boot.log 2>&1
text=$(awk '$1 == ".text" && $2 ~ /^[0-9]+$/ {s=$2} END {print s}' /tmp/bisect-boot.log)
# Report the harness's OWN gate verdict.  Recomputing it here from private
# constants silently disagreed with setup.ld's `_end <= 0x8000` assert: the old
# formula modelled _end as `1166 + .text + 30` against a 24576 limit, which
# understated the real _end by ~7 KB (measured: .text=22890 modelled 24086,
# while nm reports _end=31168) and so could print PASS for a revision that
# really overflows the gate -- the one error a size bisection must not make.
# Only the linker knows _end; only the harness prints the verdict.
gate=$(grep -m1 -E '^32 KiB gate: (PASS|FAIL) ' /tmp/bisect-boot.log || true)
if [[ -z $gate ]]; then
  echo "rev=$REV NO-GATE: the harness did not report its gate line" >&2
  tail -5 /tmp/bisect-boot.log >&2
  exit 1
fi
end=$(sed -nE 's/.*_end=([0-9]+).*/\1/p' <<<"$gate")
echo "rev=$REV text=${text:-NA} _end=${end:-NA} $gate"
