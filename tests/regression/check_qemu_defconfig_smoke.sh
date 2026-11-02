#!/usr/bin/env bash
# Exercise the verdict with a fake QEMU: no guest boot is claimed here.
set -euo pipefail
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT
mkdir -p "$work/firmware" "$work/bin"
for f in bios-256k.bin linuxboot_dma.bin kvmvapic.bin efi-e1000.rom; do : > "$work/firmware/$f"; done
: > "$work/image"
# Verdict isolation: no installed BusyBox, cpio or QEMU firmware is needed.
# These stand-ins do not run an initramfs or claim to test its bytes.
printf '#!/bin/sh\necho "fake: statically linked"\n' > "$work/bin/file"
printf '#!/bin/sh\ncat >/dev/null; printf archive\n' > "$work/bin/cpio"
printf '#!/bin/sh\ncat\n' > "$work/bin/gzip"
printf '#!/bin/sh\nexit 0\n' > "$work/bin/busybox"
chmod +x "$work/bin/"*
cat > "$work/qemu" <<'FAKE'
#!/usr/bin/env bash
set -euo pipefail
log=
while (($#)); do
  if [[ $1 == -serial ]]; then log=${2#file:}; shift 2; else shift; fi
done
[[ -n "$log" ]]
case ${FAKE_QEMU_CASE:-pass} in
  no-log) exit 1 ;;
  missing) echo 'LCCC_DEFCFG_BOOT_BEGIN' > "$log" ;;
  *) cat > "$log" <<'LOG'
LCCC_DEFCFG_BOOT_BEGIN
Linux version test
LCCC_DEFCFG_CPUS=2
LCCC_DEFCFG_SERIAL=the quick brown fox 0123456789 !?
LCCC_DEFCFG_BOOT_PASS
reboot: Power down
LOG
esac
[[ ${FAKE_QEMU_CASE:-pass} != panic ]] || echo 'Kernel panic - synthetic' >> "$log"
[[ ${FAKE_QEMU_CASE:-pass} != exit ]] || exit 1
FAKE
chmod +x "$work/qemu"
export QEMU="$work/qemu" QEMU_DATA_DIR="$work/firmware" BOOT_LOG="$work/serial" TMPDIR="$work"
export BUSYBOX="$work/bin/busybox" PATH="$work/bin:$PATH"
# An attacker-controlled output symlink must be replaced, not followed.
printf 'do not overwrite\n' > "$work/victim"
ln -s "$work/victim" "$BOOT_LOG"
FAKE_QEMU_CASE=pass "$root/scripts/qemu_defconfig_smoke.sh" "$work/image" >/dev/null
[[ ! -L "$BOOT_LOG" && $(cat "$work/victim") == 'do not overwrite' ]] || {
  echo 'FAIL: output symlink followed' >&2; exit 1;
}
for case in missing no-log panic exit; do
  if FAKE_QEMU_CASE=$case "$root/scripts/qemu_defconfig_smoke.sh" "$work/image" >"$work/out" 2>&1; then
    echo "FAIL: QEMU smoke accepted $case" >&2; cat "$work/out" >&2; exit 1
  fi
done
if "$root/scripts/qemu_defconfig_smoke.sh" "$work/absent" >/dev/null 2>&1; then
  echo 'FAIL: missing image accepted' >&2; exit 1
fi
echo 'qemu defconfig smoke verdict: PASS (fake QEMU; not a guest boot)'
