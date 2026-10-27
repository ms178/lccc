#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
source tools/linker/oracle_versions.sh
oracle_version_matches 'GNU ld (GNU Binutils) 2.47' 2.47
oracle_version_matches 'GNU ld (GNU Binutils) 2.47.20260726' 2.47
oracle_version_matches 'mold 2.42.1 (compatible with GNU ld)' 2.42.1
oracle_version_matches 'mold 3.0.0 (8de38c35a2df16a25f7ff87ac3ad07156a925beb; compatible with GNU ld)' 3.0.0
if oracle_version_matches 'mold 3.0.0 (compatible with GNU ld)' 2.42.1; then
    echo "FAIL: mold 3.0.0 accepted as the superseded 2.42.1 pin"; exit 1
fi
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
layout='new-dtags=1;sysroot=/;lib-path=/lib/example'
if oracle_native_layout_matches "$td/layout" "$layout"; then
    echo 'FAIL: absent native-layout stamp accepted'; exit 1
fi
printf '%s\n' 'new-dtags=0;sysroot=;lib-path=/lib/example' > "$td/layout"
if oracle_native_layout_matches "$td/layout" "$layout"; then
    echo 'FAIL: stale native-layout configuration accepted'; exit 1
fi
printf '%s\n' "$layout" > "$td/layout"
oracle_native_layout_matches "$td/layout" "$layout"
if oracle_native_layout_matches "$td/layout" "$layout:/other"; then
    echo 'FAIL: changed native library path accepted'; exit 1
fi
echo 'linker oracle versions: PASS'
