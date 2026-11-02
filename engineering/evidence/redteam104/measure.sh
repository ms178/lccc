#!/bin/bash
set -euo pipefail
source /home/user/.cache/redteam94/env.sh
R=/home/user/.cache/redteam104
B=/home/user/.cache/redteam103
SRC=/home/user/.cache/redteam94/zstd97/zstd-1.5.7
cd /home/user/lccc
python3 engineering/evidence/redteam88-measurements/tools/zstd_matrix.py --repo /home/user/lccc --source "$SRC" --driver "candidate=$R/driver-candidate" --driver "main=$B/driver-main" --driver "gcc=$B/driver-gcc" --baseline main --reference-cli "$B/zstd-gcc/programs/zstd" --out "$R/zstd96" --reps 9 > "$R/zstd96.log" 2>&1
valgrind --tool=memcheck --error-exitcode=99 --log-file="$R/mask-memcheck.txt" "$R/candidate-O2" > "$R/mask-memcheck.stdout"
valgrind --tool=memcheck --error-exitcode=99 --log-file="$R/zstd-memcheck.txt" "$R/driver-candidate" "$R/zstd96/inputs/source" 3 0 0 1 4 0 /dev/null "$R/candidate.zst" > "$R/zstd-memcheck.stdout"
valgrind --tool=callgrind --error-exitcode=99 --log-file="$R/zstd-callgrind.txt" --callgrind-out-file="$R/zstd.callgrind" "$R/driver-candidate" "$R/zstd96/inputs/source" 3 0 0 1 4 0 /dev/null "$R/candidate.zst" > "$R/zstd-callgrind.stdout"
printf 'OPTIMIZED MATRIX AND INSTRUMENTATION COMPLETE\n'
