#!/usr/bin/env bash
# Controlled LLVM PR 227303 comparison: identical build configuration at its
# exact merge base and head, not a stock Clang relabeled as a patched build.
set -euo pipefail
base=a1d36a0e01751ae7d8e427f667e0a62d2d59e9c5
head=a8eea1784d01b2a54b6383a8f7cf87507ce0e4e1
prefix=${1:?usage: ci_build_llvm_slp_oracles.sh PREFIX WORKDIR}
work=${2:?usage: ci_build_llvm_slp_oracles.sh PREFIX WORKDIR}
mkdir -p "$prefix" "$work"
prefix=$(cd "$prefix" && pwd)
work=$(cd "$work" && pwd)
if [[ -f $prefix/SHA256SUMS ]]; then
    (cd "$prefix" && sha256sum --check SHA256SUMS)
    python3 - "$prefix/build.json" "$base" "$head" <<'PY'
import json, sys
r = json.load(open(sys.argv[1]))
assert r['base'] == sys.argv[2] and r['head'] == sys.argv[3]
assert r['configuration'] == 'MinSizeRel;X86;assertions;Werror;j2'
PY
    exit 0
fi
[[ $(gcc-16 -dumpfullversion) == 16.2.0 ]]
[[ $(g++-16 -dumpfullversion) == 16.2.0 ]]
gcc_prefix=$(dirname "$(dirname "$(command -v gcc-16)")")
# The optional experiment needs more disk than the constrained local sandbox;
# run it on a hosted runner, with the same bounded two-job policy and swap.
if (( $(awk '/SwapTotal:/ {print $2}' /proc/meminfo) < 4194304 )); then
    echo "LLVM comparison requires active 4 GiB swap; run scripts/ci_enable_swap.sh first" >&2
    exit 1
fi
if [[ ! -d $work/source/.git ]]; then
    git init "$work/source"
    git -C "$work/source" remote add origin https://github.com/llvm/llvm-project.git
fi
[[ $(git -C "$work/source" remote get-url origin) == https://github.com/llvm/llvm-project.git ]]
git -C "$work/source" config remote.origin.promisor true
git -C "$work/source" config remote.origin.partialclonefilter blob:none
git -C "$work/source" sparse-checkout set llvm clang cmake third-party
for ref in "$base" "$head"; do
    git -C "$work/source" fetch --filter=blob:none --depth=1 origin "$ref"
done
for arm in base patched; do
    ref=$base
    [[ $arm == base ]] || ref=$head
    git -C "$work/source" checkout --detach "$ref"
    [[ $(git -C "$work/source" rev-parse HEAD) == "$ref" ]]
    [[ -z $(git -C "$work/source" status --porcelain) ]]
    cmake -S "$work/source/llvm" -B "$work/build" -G Ninja \
        -DCMAKE_BUILD_TYPE=MinSizeRel \
        -DCMAKE_C_COMPILER="$(command -v gcc-16)" \
        -DCMAKE_CXX_COMPILER="$(command -v g++-16)" \
        -DCMAKE_EXE_LINKER_FLAGS="-fuse-ld=mold -Wl,-rpath,$gcc_prefix/lib64" \
        -DLLVM_ENABLE_PROJECTS=clang -DLLVM_TARGETS_TO_BUILD=X86 \
        -DLLVM_ENABLE_ASSERTIONS=ON -DLLVM_ENABLE_WERROR=ON \
        -DLLVM_INCLUDE_TESTS=OFF -DCLANG_INCLUDE_TESTS=OFF \
        -DLLVM_INCLUDE_BENCHMARKS=OFF -DLLVM_INCLUDE_EXAMPLES=OFF \
        2>&1 | tee "$prefix/configure-$arm.log"
    cmake --build "$work/build" --target clang clang-resource-headers --parallel 2 \
        2>&1 | tee "$prefix/build-$arm.log"
    if [[ $arm == base ]]; then
        resource=$("$work/build/bin/clang" -print-resource-dir)
        mkdir -p "$prefix/resource"
        cp -a "$resource/include" "$prefix/resource/"
    fi
    install -m 755 "$work/build/bin/clang" "$prefix/clang-$arm"
    "$prefix/clang-$arm" --version > "$prefix/version-$arm.txt"
done
python3 - "$prefix/build.json" "$base" "$head" <<'PY'
import json, subprocess, sys
from pathlib import Path
r = dict(base=sys.argv[2], head=sys.argv[3],
         configuration='MinSizeRel;X86;assertions;Werror;j2',
         gcc=subprocess.check_output(['gcc-16','--version'], text=True),
         gxx=subprocess.check_output(['g++-16','--version'], text=True))
Path(sys.argv[1]).write_text(json.dumps(r, indent=2) + '\n')
PY
(cd "$prefix" && {
    sha256sum clang-base clang-patched build.json
    find resource -type f -print0 | sort -z | xargs -0 sha256sum
} > SHA256SUMS)
