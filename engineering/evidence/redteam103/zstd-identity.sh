#!/bin/bash
set -euo pipefail
source /home/user/.cache/redteam94/env.sh
R=/home/user/.cache/redteam103
SRC=/home/user/.cache/redteam94/zstd97/zstd-1.5.7
cd /home/user/lccc
for arm in candidate main gcc; do
 case "$arm" in candidate) CC="$R/candidate";; main) CC="$R/main2f47";; gcc) CC=/usr/bin/gcc-16;; esac
 B="$R/zstd-$arm"
 cmake -S "$SRC/build/cmake" -B "$B" -G Ninja -DCMAKE_C_COMPILER="$CC" -DCMAKE_CXX_COMPILER=/usr/bin/g++-16 -DCMAKE_ASM_COMPILER=/usr/bin/gcc-16 -DCMAKE_BUILD_TYPE=Release -DCMAKE_C_FLAGS="-std=gnu17 -O3 -march=x86-64-v3 -fwrapv -fno-fast-math -ffp-contract=off -I/usr/lib/gcc/x86_64-linux-gnu/16/include" -DCMAKE_C_FLAGS_RELEASE=-DNDEBUG -DZSTD_BUILD_SHARED=OFF -DZSTD_BUILD_STATIC=ON -DZSTD_PROGRAMS_LINK_SHARED=OFF -DZSTD_MULTITHREAD_SUPPORT=ON -DZSTD_LEGACY_SUPPORT=ON -DZSTD_ZLIB_SUPPORT=ON -DZSTD_LZMA_SUPPORT=ON -DZSTD_LZ4_SUPPORT=ON -DZSTD_BUILD_TESTS=OFF > "$R/zstd-$arm-configure.log" 2>&1
 cmake --build "$B" --parallel 2 --target libzstd_static zstd > "$R/zstd-$arm-build.log" 2>&1
 gcc -std=gnu17 -O2 -Werror -I"$SRC/lib" engineering/evidence/redteam88-measurements/tools/zstd_driver.c "$B/lib/libzstd.a" -pthread -o "$R/driver-$arm"
 echo "ZSTD BUILT $arm"
done
python3 - <<'PY'
from pathlib import Path
import json,hashlib
r=Path('/home/user/.cache/redteam103');o=Path('/home/user/.cache/redteam94/zstd97');sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest();rows=[]
for arm in ['candidate','main','gcc']:
 prior='candidate' if arm=='main' else arm
 for kind,p,q in [('driver',r/('driver-'+arm),o/('driver-'+prior)),('cli',r/('zstd-'+arm)/'programs/zstd',o/('build-'+prior)/'programs/zstd')]:
  row={'arm':arm,'kind':kind,'path':str(p),'sha256':sha(p),'prior_timed_arm':prior,'prior_sha256':sha(q)};row['identical_to_prior']=row['sha256']==row['prior_sha256'];rows.append(row);print(row,flush=True)
report={'complete':True,'metric':'executable identity, not new timing','explicit_language':'gnu17','rows':rows,'candidate_main_driver_identical':sha(r/'driver-candidate')==sha(r/'driver-main'),'candidate_main_cli_identical':sha(r/'zstd-candidate/programs/zstd')==sha(r/'zstd-main/programs/zstd')}
(r/'zstd-identity.json').write_text(json.dumps(report,indent=2))
PY
