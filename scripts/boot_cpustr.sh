#!/usr/bin/env bash
# Real boot CPU-string data, generated with Linux's own feature-table script.
# Source this helper and call from the kernel root. Only mkcpustr is compiled
# with HOSTCC; it is a host generator, never kernel target code.
lccc_boot_generate_cpustr() {
  local out=${1:?output directory required} hostcc=${2:?HOSTCC required}
  local capflags cpufeatures vmxfeatures mkcapflags generated status
  # mkcpustr includes capflags.c, which `make prepare` alone need not generate.
  # Use the kernel's own generator and real feature headers, never a stub.
  capflags=arch/x86/kernel/cpu/capflags.c
  cpufeatures=arch/x86/include/asm/cpufeatures.h
  vmxfeatures=arch/x86/include/asm/vmxfeatures.h
  mkcapflags=arch/x86/kernel/cpu/mkcapflags.sh
  if [[ ! -f "$capflags" || "$cpufeatures" -nt "$capflags" || \
        "$vmxfeatures" -nt "$capflags" || "$mkcapflags" -nt "$capflags" ]]; then
    generated=$(mktemp "${capflags}.lccc.XXXXXX") || return
    if sh "$mkcapflags" "$generated" "$cpufeatures" "$vmxfeatures"; then
      mv -f -- "$generated" "$capflags" || return
    else
      status=$?
      rm -f -- "$generated"
      return "$status"
    fi
  fi
  # Refresh stale data too. Publish only a successful generator's complete output.
  if [[ ! -f arch/x86/boot/cpustr.h || "$capflags" -nt arch/x86/boot/cpustr.h || \
        arch/x86/boot/mkcpustr.c -nt arch/x86/boot/cpustr.h ]]; then
    "$hostcc" -O2 -Iarch/x86/include -Iarch/x86/include/generated -Iinclude \
        arch/x86/boot/mkcpustr.c -o "$out/mkcpustr" || return
    # Stage beside the destination: OUT may be on another filesystem, where
    # mv would copy rather than atomically rename a completed header.
    generated=$(mktemp "arch/x86/boot/cpustr.h.lccc.XXXXXX") || return
    if "$out/mkcpustr" > "$generated"; then
      mv -f -- "$generated" arch/x86/boot/cpustr.h || return
    else
      status=$?
      rm -f -- "$generated"
      return "$status"
    fi
  fi
}
