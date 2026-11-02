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
cat > "$work/bin/gen_init_cpio" <<'ARCHIVE'
#!/usr/bin/env bash
set -euo pipefail
[[ $1 == -t && $2 == 0 && $# -eq 3 ]]
grep -qx 'nod /dev/console 600 0 0 c 5 1' "$3"
grep -qx 'nod /dev/null 666 0 0 c 1 3' "$3"
[[ -f bin/busybox && -f init ]]
cp init "$CAPTURE_INIT"
printf archive
ARCHIVE
chmod +x "$work/bin/"*
export GEN_INIT_CPIO="$work/bin/gen_init_cpio" CAPTURE_INIT="$work/captured-init"
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
Linux version test (lccc synthetic compiler identity)
LCCC_DEFCFG_CPUS=2
LCCC_DEFCFG_SERIAL=the quick brown fox 0123456789 !?
LCCC_DEFCFG_MEMORY_IO_PASS
LCCC_DEFCFG_BOOT_PASS
reboot: Power down
LOG
esac
[[ ${FAKE_QEMU_CASE:-pass} != wrong-compiler ]] || sed -i 's/lccc synthetic/gcc synthetic/' "$log"
[[ ${FAKE_QEMU_CASE:-pass} != panic ]] || echo 'Kernel panic - synthetic' >> "$log"
[[ ${FAKE_QEMU_CASE:-pass} != runtime-warning ]] || echo 'WARNING: synthetic kernel warning' >> "$log"
[[ ${FAKE_QEMU_CASE:-pass} != missing-workload ]] || sed -i '/LCCC_DEFCFG_MEMORY_IO_PASS/d' "$log"
[[ ${FAKE_QEMU_CASE:-pass} != console ]] || echo 'Warning: unable to open an initial console.' >> "$log"
[[ ${FAKE_QEMU_CASE:-pass} != worker-failure ]] || echo 'LCCC_SMOKE_FAIL_WORKER' >> "$log"
[[ ${FAKE_QEMU_CASE:-pass} != wrong-cpus ]] || sed -i 's/CPUS=2/CPUS=1/' "$log"
[[ ${FAKE_QEMU_CASE:-pass} != wrong-serial ]] || sed -i 's/quick brown fox/quick brown dog/' "$log"
[[ ${FAKE_QEMU_CASE:-pass} != no-powerdown ]] || sed -i '/reboot: Power down/d' "$log"
[[ ${FAKE_QEMU_CASE:-pass} != empty-log ]] || : > "$log"
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
# Check the actual embedded guest script's syntax, not just the fake verdict.
bash -n "$CAPTURE_INIT"
# Execute the generated worker/join logic with isolated command stubs. In
# particular, the first child's failure must not be hidden by the second's
# success. This tests status propagation, not guest I/O or kernel execution.
workers=$(sed -n '/^pids=$/,/^for pid in /p' "$CAPTURE_INIT")
[[ -n "$workers" ]]
for failure in none a b; do
  if FAIL_WORKER=$failure bash -e -c '
    dd() { [[ $FAIL_WORKER == none || "$*" != *"data-$FAIL_WORKER"* ]]; }
    sha256sum() { cat >/dev/null; }
    fail() { exit 1; }
    eval "$1"
  ' bash "$workers"; then
    [[ $failure == none ]] || { echo "FAIL: worker $failure failure hidden" >&2; exit 1; }
  else
    [[ $failure != none ]] || { echo 'FAIL: successful workers rejected' >&2; exit 1; }
  fi
done
for case in missing no-log empty-log panic exit wrong-compiler runtime-warning \
  missing-workload console worker-failure wrong-cpus wrong-serial no-powerdown; do
  if FAKE_QEMU_CASE=$case "$root/scripts/qemu_defconfig_smoke.sh" "$work/image" >"$work/out" 2>&1; then
    echo "FAIL: QEMU smoke accepted $case" >&2; cat "$work/out" >&2; exit 1
  fi
done
if "$root/scripts/qemu_defconfig_smoke.sh" "$work/absent" >/dev/null 2>&1; then
  echo 'FAIL: missing image accepted' >&2; exit 1
fi
if GEN_INIT_CPIO="$work/absent" "$root/scripts/qemu_defconfig_smoke.sh" "$work/image" >/dev/null 2>&1; then
  echo 'FAIL: missing archive generator accepted' >&2; exit 1
fi
printf '#!/bin/sh\nprintf partial-archive\nexit 1\n' > "$work/bad-generator"
chmod +x "$work/bad-generator"
if GEN_INIT_CPIO="$work/bad-generator" "$root/scripts/qemu_defconfig_smoke.sh" "$work/image" >/dev/null 2>&1; then
  echo 'FAIL: failed archive generator accepted' >&2; exit 1
fi
mkdir -p "$work/tmp with spaces"
TMPDIR="$work/tmp with spaces" FAKE_QEMU_CASE=pass \
  "$root/scripts/qemu_defconfig_smoke.sh" "$work/image" >/dev/null
# Exercise default host-tool discovery, not only the explicit test override.
mkdir -p "$work/kernel/arch/x86/boot" "$work/kernel/usr"
: > "$work/kernel/arch/x86/boot/bzImage"
ln -s "$work/bin/gen_init_cpio" "$work/kernel/usr/gen_init_cpio"
( cd "$work/kernel" && GEN_INIT_CPIO= FAKE_QEMU_CASE=pass \
    "$root/scripts/qemu_defconfig_smoke.sh" arch/x86/boot/bzImage >/dev/null )
ln -s "$work/kernel/arch/x86/boot/bzImage" "$work/image-symlink"
GEN_INIT_CPIO= FAKE_QEMU_CASE=pass \
  "$root/scripts/qemu_defconfig_smoke.sh" "$work/image-symlink" >/dev/null
# Exercise the real staging helper against hermetic Debian split directories.
# Nothing from the machine's own /usr/share or /usr/lib may satisfy these tests.
source "$root/scripts/qemu_firmware.sh"
mkdir -p "$work/sysroot/usr/share/qemu" "$work/sysroot/usr/share/seabios" \
  "$work/sysroot/usr/lib/ipxe/qemu" "$work/staged" "$work/missing-stage"
for f in linuxboot_dma.bin kvmvapic.bin; do
  printf '%s' "$f" > "$work/sysroot/usr/share/qemu/$f"
done
printf bios > "$work/sysroot/usr/share/seabios/bios-256k.bin"
printf ipxe > "$work/sysroot/usr/lib/ipxe/qemu/efi-e1000.rom"
QEMU_DATA_DIR= lccc_qemu_stage_firmware "$work/staged" "$work/sysroot"
[[ $(cat "$work/staged/efi-e1000.rom") == ipxe ]]
[[ $(cat "$work/staged/bios-256k.bin") == bios ]]
rm "$work/sysroot/usr/lib/ipxe/qemu/efi-e1000.rom"
if QEMU_DATA_DIR= lccc_qemu_stage_firmware "$work/missing-stage" "$work/sysroot" >"$work/missing-out" 2>&1; then
  echo 'FAIL: firmware staging accepted an absent iPXE ROM' >&2; exit 1
fi
grep -q 'missing QEMU firmware: efi-e1000.rom' "$work/missing-out"
# Also pin the explicit provider in both install paths, even though the current
# Debian dependency graph already installs it transitively. No guest boot is
# claimed by these dependency and layout checks.
install_paths=0
while IFS= read -r install; do
  [[ " $install " == *' ipxe-qemu '* ]] || {
    echo 'FAIL: kernel QEMU installation omits explicit ipxe-qemu firmware' >&2
    exit 1
  }
  install_paths=$((install_paths + 1))
done < <(grep -E 'apt-get .*install .*qemu-system-x86' "$root/.github/workflows/kernel-61858.yml")
[[ $install_paths -eq 2 ]] || {
  echo "FAIL: expected both kernel QEMU installation paths, found $install_paths" >&2
  exit 1
}
echo 'qemu defconfig smoke verdict and explicit firmware dependencies: PASS (fake QEMU; not a guest boot)'
