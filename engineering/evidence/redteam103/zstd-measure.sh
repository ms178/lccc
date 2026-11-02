#!/bin/bash
set -euo pipefail
source /home/user/.cache/redteam94/env.sh
R=/home/user/.cache/redteam103
SRC=/home/user/.cache/redteam94/zstd97/zstd-1.5.7
cd /home/user/lccc
python3 engineering/evidence/redteam88-measurements/tools/zstd_matrix.py --repo /home/user/lccc --source "$SRC" --driver "candidate=$R/driver-candidate" --driver "main=$R/driver-main" --driver "gcc=$R/driver-gcc" --baseline main --reference-cli "$R/zstd-gcc/programs/zstd" --out "$R/zstd96" --reps 9 > "$R/zstd96.log" 2>&1
for arm in candidate main gcc; do
 valgrind --tool=memcheck --error-exitcode=99 --log-file="$R/zstd-$arm-memcheck.txt" "$R/driver-$arm" "$R/zstd96/inputs/source" 3 0 0 1 4 0 /dev/null "$R/$arm.zst" > "$R/zstd-$arm-memcheck.stdout"
 valgrind --tool=callgrind --error-exitcode=99 --log-file="$R/zstd-$arm-callgrind.txt" --callgrind-out-file="$R/zstd-$arm.callgrind" "$R/driver-$arm" "$R/zstd96/inputs/source" 3 0 0 1 4 0 /dev/null "$R/$arm.zst" > "$R/zstd-$arm-callgrind.stdout"
done
printf 'ZSTD96 AND VALGRIND COMPLETE\n'
