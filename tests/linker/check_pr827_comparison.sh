#!/usr/bin/env bash
# Historical three-arm research contract; shared by hosted CI and opt-in local CI.
# Never switches branches. Baseline source trees are immutable git archives.
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"
test "$(gcc-16 -dumpfullversion)" = 16.2.0
rustc +1.99.0 --version
command -v strace
command -v valgrind
mkdir -p work target/contrast
scratch=$(mktemp -d "$root/work/pr827.XXXXXX")
trap 'rm -rf "$scratch"' EXIT
mkdir -p "$scratch/agent-b" "$scratch/prior-ours"
# Container UID differs from checkout's owner. Trust ONLY this known checkout
# for these read-only commands; never use safe.directory='*' or global mutation.
git -c "safe.directory=$root" archive cf271b0 | tar -x -C "$scratch/agent-b"
git -c "safe.directory=$root" archive fe84066 | tar -x -C "$scratch/prior-ours"
printf 'agent-b=%s\nprior-ours=%s\ncandidate=%s\n' "$(git -c "safe.directory=$root" rev-parse cf271b0)" "$(git -c "safe.directory=$root" rev-parse fe84066)" "$(git -c "safe.directory=$root" rev-parse HEAD)" > target/contrast/sources.txt
rustc +1.99.0 --version >> target/contrast/sources.txt
gcc-16 --version >> target/contrast/sources.txt
strace --version >> target/contrast/sources.txt
valgrind --version >> target/contrast/sources.txt
export CARGO_BUILD_JOBS=2
export CARGO_INCREMENTAL=0
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=gcc-16
for label in agent-b prior-ours candidate; do
    # Cargo's mtime fingerprints can reuse an archive's older files across
    # identical package IDs. Each causal arm requires its OWN build directory.
    export CARGO_TARGET_DIR="$root/target/contrast/build-$label"
    source="$scratch/$label"
    if [[ "$label" == candidate ]]; then source="$root"; fi
    cargo +1.99.0 build --manifest-path "$source/Cargo.toml" --profile fastbuild --locked -j 2 --bin lccc --bin lccc-riscv
    mkdir -p "target/contrast/$label"
    cp "$CARGO_TARGET_DIR/fastbuild/lccc" "$CARGO_TARGET_DIR/fastbuild/lccc-riscv" "target/contrast/$label/"
done
python3 tests/linker/compare_pr827.py --agent-b target/contrast/agent-b --prior-ours target/contrast/prior-ours --candidate target/contrast/candidate --cc gcc-16 --output target/contrast/results
