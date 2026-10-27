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
layout='native-wrapper-v1;new-dtags=1;sysroot=/;lib-path=/lib/example'
if oracle_native_layout_matches "$td/layout" "$layout"; then
    echo 'FAIL: absent native-layout stamp accepted'; exit 1
fi
printf '%s\n' 'new-dtags=1;sysroot=/;lib-path=/lib/example' > "$td/layout"
if oracle_native_layout_matches "$td/layout" "$layout"; then
    echo 'FAIL: legacy raw-binary layout stamp accepted as a native wrapper'; exit 1
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

source tools/linker/native_bfd_wrapper.sh
cat > "$td/real-ld" <<'SH'
#!/usr/bin/env bash
printf '%s\0' "$@" > "$CAPTURE"
SH
chmod +x "$td/real-ld"
write_native_bfd_wrapper "$td/ld-native" "$td/real-ld" \
    '/usr/local/lib:/lib/x86_64-linux-gnu'

CAPTURE="$td/args" "$td/ld-native" -rpath /opt/test
mapfile -d '' -t got < "$td/args"
[[ ${got[0]} == --sysroot=/ && ${got[1]} == --enable-new-dtags ]] || {
    echo 'FAIL: native wrapper defaults missing'; exit 1;
}
[[ ${got[2]} == -rpath && ${got[3]} == /opt/test ]] || {
    echo 'FAIL: native wrapper changed caller arguments'; exit 1;
}
[[ ${got[4]} == -L/usr/local/lib && ${got[5]} == -L/lib/x86_64-linux-gnu ]] || {
    echo 'FAIL: native library search paths missing or reordered'; exit 1;
}

CAPTURE="$td/args" "$td/ld-native" --disable-new-dtags --sysroot /tmp/root \
    '-L/custom dir' -lc
mapfile -d '' -t got < "$td/args"
[[ ${got[2]} == --disable-new-dtags && ${got[3]} == --sysroot &&
   ${got[4]} == /tmp/root && ${got[5]} == '-L/custom dir' && ${got[6]} == -lc ]] || {
    echo 'FAIL: explicit BFD defaults or spaced arguments were not preserved'; exit 1;
}
[[ ${got[7]} == -L/tmp/root/usr/local/lib &&
   ${got[8]} == -L/tmp/root/lib/x86_64-linux-gnu ]] || {
    echo 'FAIL: native search paths ignored the caller sysroot'; exit 1;
}

CAPTURE="$td/args" "$td/ld-native" -nostdlib -L/custom -lc
mapfile -d '' -t got < "$td/args"
for arg in "${got[@]}"; do
    case $arg in
        -L/usr/local/lib|-L/lib/x86_64-linux-gnu)
            echo 'FAIL: -nostdlib received injected native search paths'; exit 1 ;;
    esac
done
echo 'linker oracle versions: PASS'
