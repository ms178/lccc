#!/usr/bin/env bash
# Stage real QEMU/SeaBIOS/iPXE firmware into a private directory. The optional
# sysroot is for hermetic host-layout tests; production uses the real root.
lccc_qemu_stage_firmware() {
  local destination=${1:?firmware destination required} sysroot=${2:-/}
  local firmware directory found
  # Four of these five are what SeaBIOS and the PXE path need. The fifth,
  # vgabios-stdvga.bin, is what `-machine pc` needs: its default display is
  # `-vga std`, and QEMU resolves that ROM FROM THE -L DIRECTORY, which replaces
  # its compiled-in data dir rather than extending it. Staging only the first
  # four made qemu_defconfig_smoke.sh die before the guest started --
  #   qemu-system-x86_64: failed to find romfile "vgabios-stdvga.bin"
  # -- leaving a zero-byte serial log whose first missing marker read like a
  # kernel defect. The prefix always ships the file (qemu-system-data and
  # seabios both do), so it is required, not best-effort.
  #
  # These five names are the firmware-completeness contract shared with
  # ensure_qemu_system.sh (FIRMWARE), kernel_isolate.sh, qemu_boot_test.sh and
  # qemu_qmp_probe.py; change the list in all of them or not at all.
  for firmware in bios-256k.bin linuxboot_dma.bin kvmvapic.bin efi-e1000.rom \
                  vgabios-stdvga.bin; do
    found=0
    for directory in ${QEMU_DATA_DIR:+"$QEMU_DATA_DIR"} \
      "${sysroot%/}/usr/share/qemu" "${sysroot%/}/usr/share/seabios" \
      "${sysroot%/}/usr/lib/ipxe/qemu"; do
      if [[ -f "$directory/$firmware" ]]; then
        ln -s "$directory/$firmware" "$destination/$firmware" || return 1
        found=1
        break
      fi
    done
    if (( ! found )); then
      echo "missing QEMU firmware: $firmware" >&2
      return 1
    fi
  done
}
