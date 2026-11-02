#!/usr/bin/env bash
# Setup-size probes must never replace Kbuild's real generated offsets.
set -euo pipefail
repo=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
tmp=$(mktemp -d)
trap 'rm -rf -- "$tmp"' EXIT
mkdir -p "$tmp/tree/arch/x86/boot" "$tmp/stage"
printf '#include "zoffset.h"\n#include "voffset.h"\n' > "$tmp/tree/arch/x86/boot/header.S"
printf '#define ZO_startup_32 0x9876\n' > "$tmp/tree/arch/x86/boot/zoffset.h"
printf '#define VO__text 0x7654\n#define VO__end 0x7655\n' > "$tmp/tree/arch/x86/boot/voffset.h"
cd "$tmp/tree"
before=$(sha256sum arch/x86/boot/{header.S,zoffset.h,voffset.h})
# shellcheck source=../../scripts/boot_offsets.sh
source "$repo/scripts/boot_offsets.sh"
lccc_boot_stage_header "$tmp/stage"
[[ $(sha256sum arch/x86/boot/{header.S,zoffset.h,voffset.h}) == "$before" ]]
cmp arch/x86/boot/header.S "$tmp/stage/header.S"
grep -q '^#define ZO_efi32_stub_entry ' "$tmp/stage/zoffset.h"
grep -q '^#define VO__text 0x100000$' "$tmp/stage/voffset.h"
# Prove quoted include resolution favors the staged offsets, not tree files.
printf '#include "zoffset.h"\nZO_startup_32\n' > "$tmp/stage/probe.c"
gcc -E -P -I arch/x86/boot "$tmp/stage/probe.c" | grep -qx '0x1000'
echo 'PASS: private boot offset staging leaves Kbuild offsets untouched'
