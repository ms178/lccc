#!/usr/bin/env bash
# Measure the 32 KiB boot-gate .text size for a given lccc commit.
# Usage: bisect_boot.sh <git-rev>
set -uo pipefail
REV=$1
WT=/home/user/bisect
REPO=/home/user/lccc
K=/home/user/kernel-work/linux-6.18.50
export PATH=/home/user/.cargo/bin:$PATH
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
content=$(( 1166 + ${text:-0} + 30 ))
echo "rev=$REV text=${text:-NA} content_end=$content $( [ "$content" -le 24576 ] && echo PASS || echo FAIL )"
