#!/usr/bin/env bash
# Stage real QEMU/SeaBIOS/iPXE firmware into a private directory. The optional
# sysroot is for hermetic host-layout tests; production uses the real root.
lccc_qemu_stage_firmware() {
  local destination=${1:?firmware destination required} sysroot=${2:-/}
  local firmware directory found
  for firmware in bios-256k.bin linuxboot_dma.bin kvmvapic.bin efi-e1000.rom; do
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
