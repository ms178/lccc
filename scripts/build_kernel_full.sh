#!/usr/bin/env bash
# ============================================================================
# build_kernel_full.sh — full Linux/CachyMod build with LCCC for ALL target
# code and lccc-ld for ALL target links. Host tools (including the real
# objtool) use HOSTCC. No validation or security feature is disabled by this
# harness, and a no-op objtool shim is rejected outright.
#
# Configuration contract:
#   * An existing .config is preserved by default. LCCC_RECONFIG=1 explicitly
#     selects the package configuration (or a documented defconfig recipe when
#     no package config is available). A missing BORE symbol is NOT a license
#     to overwrite a caller-selected configuration.
#   * When the package config is selected, olddefconfig may absorb new symbols
#     but must not silently drop a package-enabled feature; drift is refused
#     unless LCCC_ALLOW_CONFIG_DRIFT=1 is set explicitly.
#   * The exact configuration, compiler, linker and recipe identities are
#     recorded before the build and re-verified afterwards.
#
# Usage: build_kernel_full.sh [kbuild-targets...]   (default: bzImage)
# Environment:
#   KERNEL_DIR, KERNEL_WORK, PKGDIR, LCCC, LCCC_LD, HOSTCC, LCCC_JOBS=2,
#   LCCC_RECONFIG=0, LCCC_KERNEL_SCX=0 (opt-in BTF/sched-ext milestone),
#   LCCC_ALLOW_CONFIG_DRIFT=0.
# ============================================================================
set -euo pipefail

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
# shellcheck source=kernel_version.sh
. "$here/kernel_version.sh"
# shellcheck source=kconfig_query.sh
. "$here/kconfig_query.sh"

K=${KERNEL_DIR:-$(lccc_kernel_default_dir)}
PKGDIR=${PKGDIR:-$(lccc_kernel_pkgdir)}
REPO=$(CDPATH= cd -- "$here/.." && pwd)
LCCC=${LCCC:-$REPO/target/fastbuild/lccc}
LCCC_LD=${LCCC_LD:-$REPO/target/fastbuild/lccc-ld}
HOSTCC=${HOSTCC:-gcc}
JOBS=${LCCC_JOBS:-2}

[[ $JOBS =~ ^[1-9][0-9]*$ ]] || {
  echo 'LCCC_JOBS must be a positive integer' >&2; exit 2;
}
[[ ${LCCC_NOOBJTOOL:-0} == 0 ]] || {
  echo 'LCCC_NOOBJTOOL=1 is no longer supported: a no-op validator is not a kernel build' >&2
  exit 2
}
[[ -x $LCCC && -x $LCCC_LD ]] || {
  echo 'build LCCC first with scripts/build_lccc_fast.sh' >&2; exit 1;
}
LCCC=$(realpath -- "$LCCC")
LCCC_LD=$(realpath -- "$LCCC_LD")

# Prepare only a missing/incomplete tree. Re-running preparation is not a
# license to overwrite an existing caller-selected configuration; the prepare
# script's recipe/ownership guards decide whether a damaged tree may be replaced.
if [[ ! -f $K/.lccc-prepared || ! -f $K/include/generated/autoconf.h ]]; then
  if [[ -n $PKGDIR ]]; then
    PKGDIR=$PKGDIR "$here/prepare_kernel_tree.sh" "$K"
  else
    "$here/prepare_kernel_tree.sh" "$K"
  fi
fi
[[ -f $K/Makefile && -f $K/.lccc-prepared ]] || {
  echo "not a prepared kernel tree: $K" >&2; exit 1;
}
cd "$K"

# Kbuild does not fingerprint compiler/linker executable contents. Invalidate
# stale products before any target code is rebuilt, and keep the boot ISA
# portability fix idempotent and explicit.
"$here/kernel_tool_identity.sh" "$K" "$LCCC" "$LCCC_LD"
"$here/kernel_boot_isa.sh" patch "$K"

make_args=(ARCH=x86_64 CC="$LCCC" LD="$LCCC_LD" AS="$LCCC" HOSTCC="$HOSTCC")
package_config=""
config_recipe=existing-config
if [[ ${LCCC_RECONFIG:-0} == 1 ]] || [[ ! -f .config ]]; then
  if [[ -n $PKGDIR && -r $PKGDIR/config ]]; then
    cp -f "$PKGDIR/config" .config
    package_config=$PKGDIR/config
    config_recipe=package-config
  else
    make "${make_args[@]}" defconfig >/dev/null
    ./scripts/config \
      --enable CACHY --enable SCHED_BORE --enable SCHED_CACHE \
      --enable HZ_800 --disable HZ_1000 --enable PREEMPT --disable PREEMPT_VOLUNTARY \
      --enable TCP_CONG_BBR --enable DEBUG_FS --enable IKCONFIG --enable IKCONFIG_PROC \
      --enable BLK_DEV_INITRD --enable RD_GZIP --enable DEVTMPFS --enable DEVTMPFS_MOUNT \
      --enable SERIAL_8250 --enable SERIAL_8250_CONSOLE --enable KERNEL_ZSTD
    config_recipe=fresh-defconfig-cachymod
    if [[ ${LCCC_KERNEL_SCX:-0} == 1 ]]; then
      ./scripts/config --enable BPF_SYSCALL --enable DEBUG_INFO --enable DEBUG_INFO_DWARF5 \
        --enable DEBUG_INFO_BTF --enable SCHED_CLASS_EXT
      config_recipe+=+scx-btf
    fi
  fi
fi

make "${make_args[@]}" olddefconfig </dev/null >/dev/null
make "${make_args[@]}" syncconfig </dev/null >/dev/null

# A package config is the source of truth for the package's feature set.
# olddefconfig may absorb NEW symbols, but dropping a package-enabled feature
# means the build is no longer the package's kernel.
if [[ -n $package_config ]]; then
  symbols=(
    CONFIG_FUNCTION_TRACER CONFIG_DYNAMIC_FTRACE
    CONFIG_STACKPROTECTOR CONFIG_STACKPROTECTOR_STRONG
    CONFIG_UNWINDER_ORC CONFIG_OBJTOOL CONFIG_STACK_VALIDATION CONFIG_DEBUG_ENTRY
    CONFIG_SCHED_BORE CONFIG_SCHED_CACHE CONFIG_SCHED_CLASS_EXT
    CONFIG_CACHY CONFIG_TCP_CONG_BBR CONFIG_PREEMPT CONFIG_IKCONFIG_PROC
  )
  if ! drift=$(lccc_config_drift "$package_config" .config "${symbols[@]}"); then
    echo 'config: the package enables these features but the prepared .config does not:' >&2
    printf '%s\n' "$drift" | sed 's/^/config:   /' >&2
    if [[ ${LCCC_ALLOW_CONFIG_DRIFT:-0} == 1 ]]; then
      echo 'config: LCCC_ALLOW_CONFIG_DRIFT=1 — continuing with a different kernel' >&2
    else
      echo 'config: refusing to build a kernel the package does not ship.' >&2
      echo 'config: fix the gap, or set LCCC_ALLOW_CONFIG_DRIFT=1 to build anyway.' >&2
      exit 1
    fi
  fi
fi
if [[ ${LCCC_KERNEL_SCX:-0} == 1 ]] && ! grep -q '^CONFIG_SCHED_CLASS_EXT=y' .config; then
  echo 'config: LCCC_KERNEL_SCX=1 requested sched-ext but Kconfig did not enable it' >&2
  exit 1
fi

make "${make_args[@]}" prepare -j"$JOBS"

# Kconfig may force OBJTOOL on x86-64. Build and use the real host executable;
# never replace it with a script. The build itself remakes it when inputs change.
if grep -q '^CONFIG_OBJTOOL=y' .config; then
  make "${make_args[@]}" tools/objtool -j"$JOBS"
  # Verify the ELF magic directly instead of depending on file(1): a no-op
  # shell shim must never be accepted as the real host validator.
  [[ -x tools/objtool/objtool && $(head -c4 tools/objtool/objtool) == $'\177ELF' ]] || {
    echo 'real ELF objtool is missing' >&2; exit 1;
  }
fi

for feature in STACKPROTECTOR UNWINDER_ORC OBJTOOL FUNCTION_TRACER DYNAMIC_FTRACE \
               MITIGATION_RETPOLINE SCHED_CLASS_EXT DEBUG_INFO_BTF; do
  if grep -q "^CONFIG_${feature}=y" .config; then echo "CONFIG_${feature}=y"; fi
done

config_hash=$(sha256sum .config | cut -d' ' -f1)
compiler_hash=$(sha256sum "$LCCC" | cut -d' ' -f1)
linker_hash=$(sha256sum "$LCCC_LD" | cut -d' ' -f1)
stamp_tmp=$(mktemp .lccc-full-build-config.tmp.XXXXXX)
trap 'rm -f -- "$stamp_tmp"' EXIT
{
  printf 'recipe=%s\nconfig-sha256=%s\ncompiler-sha256=%s\nlinker-sha256=%s\n' \
    "$config_recipe" "$config_hash" "$compiler_hash" "$linker_hash"
  printf 'boot-isa-sha256=%s\n' "$(sha256sum "$here/kernel_boot_isa.sh" | cut -d' ' -f1)"
  if [[ -n $package_config ]]; then
    printf 'package-config-sha256=%s\n' "$(sha256sum "$package_config" | cut -d' ' -f1)"
  fi
  if [[ -d .git ]] || git rev-parse --git-dir >/dev/null 2>&1; then
    printf 'kernel-commit=%s\n' "$(git rev-parse HEAD 2>/dev/null || echo unknown)"
  fi
} > "$stamp_tmp"
mv -f "$stamp_tmp" .lccc-full-build-config
cp .config .lccc-full-build.config

printf 'Building with LCCC (CC=%s LD=%s AS=%s, -j%s, config=%s)\n' \
  "$LCCC" "$LCCC_LD" "$LCCC" "$JOBS" "$config_hash"
targets=("$@")
if ((${#targets[@]} == 0)); then targets=(bzImage); fi
make "${make_args[@]}" -j"$JOBS" "${targets[@]}"

# syncconfig during the build must not silently change the recorded contract,
# and a compiler/linker rebuilt underneath the build invalidates the artifacts.
[[ $(sha256sum .config | cut -d' ' -f1) == "$config_hash" ]] || {
  echo 'configuration changed during Kbuild; inspect and rerun' >&2; exit 1;
}
[[ $(sha256sum "$LCCC" | cut -d' ' -f1) == "$compiler_hash" ]] || {
  echo 'compiler changed during Kbuild; inspect and rerun' >&2; exit 1;
}
[[ $(sha256sum "$LCCC_LD" | cut -d' ' -f1) == "$linker_hash" ]] || {
  echo 'linker changed during Kbuild; inspect and rerun' >&2; exit 1;
}
