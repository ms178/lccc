#!/usr/bin/env bash
# Shared setup-only offset fallback for the x86 boot-size harnesses.
#
# A full kernel build generates zoffset.h/voffset.h from real binaries. Setup
# size probes must not overwrite those files: newer stub timestamps could make
# Kbuild reuse bogus boot offsets in a subsequent real bzImage. Stage a copy of
# header.S and private deterministic offsets instead. Other quoted includes
# still resolve through the normal -Iarch/x86/boot search path.

ensure_boot_offset_stubs() {
  local zoffset=${1:?private zoffset path required}
  local voffset=${2:?private voffset path required}
  local tmp

  if [[ ! -r "$zoffset" ]] || \
     ! grep -q '^#define ZO_efi32_stub_entry ' "$zoffset" || \
     ! grep -q '^#define ZO_efi64_stub_entry ' "$zoffset"; then
    tmp="${zoffset}.tmp.$$"
    cat >"$tmp" <<'EOF'
/* STUB zoffset.h — normally generated from compressed/vmlinux.
 * Values are immediates only; they do not affect setup.elf size. */
#define ZO_startup_32 0x1000
#define ZO_efi32_stub_entry 0x1000
#define ZO_efi64_stub_entry 0x1200
#define ZO_efi_pe_entry 0x1300
#define ZO_efi32_pe_entry 0x1400
#define ZO_input_data 0x200000
#define ZO_kernel_info 0x1500
#define ZO__end 0x800000
#define ZO__ehead 0x200
#define ZO__text 0x210
#define ZO__data 0x300
#define ZO__edata 0x800000
#define ZO__sbat 0x400
#define ZO__esbat 0x410
#define ZO_z_input_len 0x100000
#define ZO_z_output_len 0x400000
#define ZO_z_extract_offset 0x0
#define ZO_z_min_extract_offset 0x1000
#define ZO_z_extra_bytes 0x10000
#define ZO_INIT_SIZE 0x500000
EOF
    mv -f "$tmp" "$zoffset"
  fi

  # Current vmlinux-generated files already contain these symbols.  Only fill
  # a missing file: unlike zoffset.h, their values can affect header constants
  # in a real build and should not be overwritten unnecessarily.
  if [[ ! -r "$voffset" ]] || \
     ! grep -q '^#define VO__text ' "$voffset" || \
     ! grep -q '^#define VO__end ' "$voffset"; then
    tmp="${voffset}.tmp.$$"
    cat >"$tmp" <<'EOF'
/* STUB voffset.h — normally generated from vmlinux. Immediates only. */
#define VO__text 0x100000
#define VO__end  0x800000
EOF
    mv -f "$tmp" "$voffset"
  fi
}

# Called from the kernel root with a freshly created private directory.
# Keep header.S beside its offsets so quoted include lookup finds the stubs
# before the real kernel tree, regardless of -I search order.
lccc_boot_stage_header() {
  local dir=${1:?private staging directory required}
  cp -- arch/x86/boot/header.S "$dir/header.S"
  ensure_boot_offset_stubs "$dir/zoffset.h" "$dir/voffset.h"
}
