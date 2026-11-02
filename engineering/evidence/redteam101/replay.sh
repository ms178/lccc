#!/bin/bash
set -euo pipefail
source /home/user/.cache/redteam94/env.sh
R=/home/user/.cache/redteam94
REPO=/home/user/lccc
MAIN=/home/user/.cache/lccc-main97
mkdir -p "$R/performance97" "$R/arms"
cd "$MAIN"
bash scripts/build_lccc_fast.sh > "$R/performance97/main-build.log" 2>&1
cp target/fastbuild/lccc "$R/arms/main84"
cp "$REPO/target/fastbuild/lccc" "$R/arms/s98"
cd "$REPO"
python3 tests/benchmark/run_benchmarks.py --compilers lccc,gcc,clang,icx --lccc "$R/arms/s98" --gcc /usr/bin/gcc-16 --clang /usr/bin/clang-23 --icx "$R/intel/opt/intel/oneapi/compiler/2026.1/bin/icx" --candidate "main=$R/arms/main84" --opt=-O2 --cflag=-march=x86-64-v3 --cflag=-fwrapv --cflag=-fno-fast-math --cflag=-ffp-contract=off --cflag=-Wl,--no-as-needed --cflag=-lm --reps 9 --warmup 2 --artifact-dir "$R/performance97/native45" --json "$R/performance97/native45.json" --markdown "$R/performance97/native45.md" --strict > "$R/performance97/native45.log" 2>&1
python3 scripts/benchmark_comparison_report.py "$R/performance97/native45.json" --out "$R/performance97/native45-qualified.json"
SRC="$R/zstd97/zstd-1.5.7"
for arm in candidate main gcc; do
 case "$arm" in candidate) CC="$R/arms/s98";; main) CC="$R/arms/main84";; gcc) CC=/usr/bin/gcc-16;; esac
 B="$R/zstd97/build-$arm"
 cmake -S "$SRC/build/cmake" -B "$B" -G Ninja -DCMAKE_C_COMPILER="$CC" -DCMAKE_CXX_COMPILER=/usr/bin/g++-16 -DCMAKE_ASM_COMPILER=/usr/bin/gcc-16 -DCMAKE_BUILD_TYPE=Release -DCMAKE_C_FLAGS="-O3 -march=x86-64-v3 -fwrapv -fno-fast-math -ffp-contract=off -I/usr/lib/gcc/x86_64-linux-gnu/16/include" -DCMAKE_C_FLAGS_RELEASE=-DNDEBUG -DZSTD_BUILD_SHARED=OFF -DZSTD_BUILD_STATIC=ON -DZSTD_PROGRAMS_LINK_SHARED=OFF -DZSTD_MULTITHREAD_SUPPORT=ON -DZSTD_LEGACY_SUPPORT=ON -DZSTD_ZLIB_SUPPORT=ON -DZSTD_LZMA_SUPPORT=ON -DZSTD_LZ4_SUPPORT=ON -DZSTD_BUILD_TESTS=OFF > "$R/performance97/zstd-$arm-configure.log" 2>&1
 cmake --build "$B" --parallel 2 --target libzstd_static zstd > "$R/performance97/zstd-$arm-build.log" 2>&1
 gcc -O2 -Werror -I"$SRC/lib" engineering/evidence/redteam88-measurements/tools/zstd_driver.c "$B/lib/libzstd.a" -pthread -o "$R/zstd97/driver-$arm"
done
python3 engineering/evidence/redteam88-measurements/tools/zstd_matrix.py --repo "$REPO" --source "$SRC" --driver "candidate=$R/zstd97/driver-candidate" --driver "main=$R/zstd97/driver-main" --driver "gcc=$R/zstd97/driver-gcc" --baseline main --reference-cli "$R/zstd97/build-gcc/programs/zstd" --out "$R/performance97/zstd96" --reps 9 > "$R/performance97/zstd96.log" 2>&1
for arm in candidate main gcc; do
 valgrind --tool=memcheck --error-exitcode=99 --log-file="$R/performance97/zstd-$arm-memcheck.txt" "$R/zstd97/driver-$arm" "$R/performance97/zstd96/inputs/source" 3 0 0 1 4 0 /dev/null "$R/performance97/$arm.zst" > "$R/performance97/zstd-$arm-memcheck.stdout"
 valgrind --tool=callgrind --error-exitcode=99 --log-file="$R/performance97/zstd-$arm-callgrind.txt" --callgrind-out-file="$R/performance97/zstd-$arm.callgrind" "$R/zstd97/driver-$arm" "$R/performance97/zstd96/inputs/source" 3 0 0 1 4 0 /dev/null "$R/performance97/$arm.zst" > "$R/performance97/zstd-$arm-callgrind.stdout"
done
printf 'COMPLETE\n'
