#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
source tools/linker/oracle_versions.sh
oracle_version_matches 'GNU ld (GNU Binutils) 2.47' 2.47
oracle_version_matches 'GNU ld (GNU Binutils) 2.47.20260726' 2.47
oracle_version_matches 'mold 2.42.1 (compatible with GNU ld)' 2.42.1
for wrong in 'GNU ld 2.44' 'GNU ld 2.470' 'GNU ld 2.47.1' 'GNU ld x2.47' \
             'GNU ld 2.47.20260725' 'GNU ld 2.47.202607260' \
             'GNU ld 2.47.20260726-dirty' ''; do
    if oracle_version_matches "$wrong" 2.47; then
        echo "FAIL: accepted $wrong"; exit 1
    fi
done
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT
cat > "$td/ld" <<'SH'
#!/bin/sh
printf 'GNU ld (GNU Binutils) 2.44\nlicense mentions 2.47\n'
SH
chmod +x "$td/ld"
if oracle_require_version "$td/ld" 2.47 >"$td/log" 2>&1; then
    echo 'FAIL: stale restored binary accepted'; exit 1
fi
grep -q 'oracle version mismatch' "$td/log"
printf '#!/bin/sh\necho "GNU ld 2.47"; exit 7\n' > "$td/ld"
if oracle_require_version "$td/ld" 2.47 >"$td/log" 2>&1; then
    echo 'FAIL: failed executable accepted'; exit 1
fi
echo 'linker oracle versions: PASS'
