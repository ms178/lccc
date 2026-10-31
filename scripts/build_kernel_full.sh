#!/usr/bin/env bash
# ============================================================================
# build_kernel_full.sh — drive a full linux-cachymod kernel build with LCCC.
#
# This is the incremental "real workload" harness for the kernel-compilation
# quest.  It prepares the patched linux-cachymod tree (see prepare_kernel_tree.sh;
# the version is whatever the package PKGBUILD pins),
# compiles it with the package's OWN x86_64 configuration (`cp ../config
# .config`, exactly what the PKGBUILD's prepare() does: CachyOS scheduler set,
# FUNCTION_TRACER/DYNAMIC_FTRACE, STACKPROTECTOR, OBJTOOL, UNWINDER_ORC, ...),
# and drives the whole Kbuild with `CC=lccc LDLD=lccc-ld AS=lccc`.
#
# The harness is *honest about what it builds*. It no longer disables package
# features to get past a gap (tracer, stack protector and objtool/ORC all work
# now), it stops if `olddefconfig` drops a symbol the package enabled, and the
# objtool no-op shim is opt-in (LCCC_NOOBJTOOL=1) and only legitimate for
# compile-only sweeps — a boot test must run the real objtool.
#
# Usage:
#   build_kernel_full.sh [kbuild-targets...]
#     e.g. build_kernel_full.sh                          # arch/x86 defconfig build
#          build_kernel_full.sh kernel/exit.o            # one object
#          build_kernel_full.sh net/core/ fs/ ...        # selected subdirs
#
# Environment:
#   KERNEL_DIR  patched tree  (default: kernel_version.sh's answer, i.e.
#               $KERNEL_WORK/linux-$KVER under ~/.cache/lccc/kernel-work)
#   PKGDIR      archpkgbuilds pkg dir (default .../packages/linux-cachymod-6.18)
#   LCCC        compiler (default <repo>/target/fastbuild/lccc, where <repo> is
#               the checkout this script lives in)
#   LCCC_LD     linker   (default <repo>/target/fastbuild/lccc-ld)
#   LCCC_JOBS   build parallelism (default 2)
#   LCCC_NOOBJTOOL  if 1, swap tools/objtool/objtool for a no-op shim. That
#                  makes the output NOT the package's kernel (no ORC data, no
#                  stack validation), so it is for compile-only sweeps, never
#                  for a boot test. Default 0: real objtool.
#   LCCC_RECONFIG  if 1, force rewrite of .config; default reuses existing.
# ============================================================================
set -euo pipefail

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
# Which kernel tree?  scripts/kernel_version.sh is the single source of truth
# (explicit KERNEL_DIR/KVER > the prepared tree on disk > the package PKGBUILD
# > a pinned fallback); the literal that used to live here went stale the
# moment archpkgbuilds bumped 6.18.55 -> 6.18.58.
# shellcheck source=kernel_version.sh
. "$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/kernel_version.sh"
# shellcheck source=kconfig_query.sh
. "$here/kconfig_query.sh"
K=${KERNEL_DIR:-$(lccc_kernel_default_dir)}
PKGDIR=${PKGDIR:-$(lccc_kernel_pkgdir)}
REPO_ROOT=$(CDPATH= cd -- "$here/.." && pwd)
LCCC=${LCCC:-$REPO_ROOT/target/fastbuild/lccc}
LCCC_LD=${LCCC_LD:-$REPO_ROOT/target/fastbuild/lccc-ld}
HOSTCC=${HOSTCC:-gcc}
JOBS=${LCCC_JOBS:-2}
NOOBJTOOL=${LCCC_NOOBJTOOL:-0}
RECONFIG=${LCCC_RECONFIG:-0}

# ---- 1. ensure the patched tree exists -------------------------------------
if [[ -n $PKGDIR ]]; then
  PKGDIR="$PKGDIR" "$here/prepare_kernel_tree.sh" "$K"
else
  "$here/prepare_kernel_tree.sh" "$K"
fi

# ---- 2. configuration: the package's own config, verbatim -------------------
#   * PKGBUILD's prepare() does `cp ../config .config` for the package's full
#     x86_64 configuration (CachyOS scheduler set, FUNCTION_TRACER/DYNAMIC_
#     FTRACE, STACKPROTECTOR, OBJTOOL + UNWINDER_ORC, ...).  This script now
#     does exactly that: the package config is the source of truth and it is
#     copied unmodified (olddefconfig only absorbs symbols a newer tree adds).
#     Earlier revisions of this script rebuilt a defconfig with the tracer,
#     stack protector and ORC/objtool disabled as "documented prerequisites";
#     those features are implemented now (LCCC emits `call __fentry__` under
#     -pg -mfentry, byte-identical canary access under the kernel's exact
#     -mstack-protector-guard-* spelling, and real objtool accepts and ORC-
#     annotates LCCC objects), and disabling package features to make a build
#     pass is not an acceptable trade.
#   * olddefconfig absorbs NEW Kconfig symbols non-interactively (avoids the
#     "Restart config..." prompt that otherwise stalls a headless build).
cd "$K"

# Package features a silent drop would make this build a *different* kernel.
# Checked only where the package config itself enables them, so a package that
# legitimately disables one is never second-guessed.
PACKAGE_FEATURES=(
  FUNCTION_TRACER DYNAMIC_FTRACE
  STACKPROTECTOR STACKPROTECTOR_STRONG
  UNWINDER_ORC OBJTOOL STACK_VALIDATION DEBUG_ENTRY
  SCHED_BORE SCHED_CACHE SCHED_CLASS_EXT
)

if [[ $RECONFIG == 1 ]] || ! grep -q "^CONFIG_SCHED_BORE=y" .config; then
  if [[ -n $PKGDIR && -r $PKGDIR/config ]]; then
    cp -f "$PKGDIR/config" .config
    package_config=$PKGDIR/config
    echo "config: package configuration $package_config"
    echo "config: package sha256 $(sha256sum "$package_config" | cut -d' ' -f1)"
  else
    package_config=""
    echo "WARNING: no package configuration (\$PKGDIR=${PKGDIR:-unset});" >&2
    echo "WARNING: falling back to defconfig + the CachyOS scheduler enables." >&2
    echo "WARNING: this is NOT the configuration the package ships, and nothing" >&2
    echo "WARNING: below verifies the package's feature set." >&2
    make ARCH=x86_64 CC="$LCCC" LD="$LCCC_LD" HOSTCC="$HOSTCC" defconfig >/dev/null
    ./scripts/config \
      --enable CACHY \
      --enable SCHED_BORE \
      --enable SCHED_CACHE \
      --enable SCHED_CLASS_EXT
  fi
  make ARCH=x86_64 CC="$LCCC" LD="$LCCC_LD" HOSTCC="$HOSTCC" olddefconfig </dev/null >/dev/null
  make ARCH=x86_64 CC="$LCCC" LD="$LCCC_LD" HOSTCC="$HOSTCC" syncconfig </dev/null >/dev/null
  echo "config: resulting .config sha256 $(sha256sum .config | cut -d' ' -f1)"

  # olddefconfig absorbs new symbols, but it also *drops* symbols whose
  # dependencies the tree no longer meets. For a package feature that is not a
  # detail: the build would quietly stop being the package's kernel. Report the
  # whole drift at once, then refuse — a boot test of a different kernel proves
  # nothing about the package's.
  if [[ -n $package_config ]]; then
    symbols=()
    for feature in "${PACKAGE_FEATURES[@]}"; do symbols+=("CONFIG_$feature"); done
    if ! drift=$(lccc_config_drift "$package_config" .config "${symbols[@]}"); then
      echo "config: the package enables these features but the prepared .config does not:" >&2
      printf '%s\n' "$drift" | sed 's/^/config:   /' >&2
      if [[ ${LCCC_ALLOW_CONFIG_DRIFT:-0} == 1 ]]; then
        echo "config: LCCC_ALLOW_CONFIG_DRIFT=1 — continuing with a kernel that is not the package's" >&2
      else
        echo "config: refusing to build a kernel the package does not ship." >&2
        echo "config: fix the gap, or re-enable with ./scripts/config --enable <SYMBOL>," >&2
        echo "config: or set LCCC_ALLOW_CONFIG_DRIFT=1 to build anyway." >&2
        exit 1
      fi
    fi
  fi
  make ARCH=x86_64 CC="$LCCC" LD="$LCCC_LD" HOSTCC="$HOSTCC" prepare -j"$JOBS" >/dev/null
fi

# ---- 3. objtool (real by default; a no-op shim is a deliberate opt-in) -----
# x86_64 6.18 *requires* objtool (HAVE_OBJTOOL is selected for X86_64), and
# real objtool accepts and ORC-annotates LCCC objects, so it runs by default:
# shimming it silently drops UNWINDER_ORC data and stack validation, which is
# a different kernel from the one the package ships. LCCC_NOOBJTOOL=1 remains
# for compile-only sweeps, where a boot artifact is not claimed.
if [[ $NOOBJTOOL == 1 ]] && [[ -x tools/objtool/objtool ]]; then
  if ! grep -q "no-op objtool for lccc compile sweep" tools/objtool/objtool 2>/dev/null; then
    cp -f tools/objtool/objtool /tmp/objtool.real.$$
    cat > tools/objtool/objtool <<'EOF'
#!/usr/bin/env bash
# no-op objtool for LCCC compile sweep (objtool interop is a tracked blocker)
exit 0
EOF
    chmod +x tools/objtool/objtool
    printf 'note: tools/objtool/objtool shimmed to no-op (LCCC_NOOBJTOOL=1)\n' >&2
  fi
fi

# ---- 4. build -----------------------------------------------------------------
printf 'Building kernel with LCCC (CC=%s LD=%s AS=%s, -j%s)\n' "$LCCC" "$LCCC_LD" "$LCCC" "$JOBS"
targets=("$@")
if (( ${#targets[@]} == 0 )); then
  targets=(vmlinux)
fi
# `AS` (uppercase) is the Kbuild assembler variable; the previous lowercase
# `as=` was a plain (ignored) make variable, so .S files silently used the host
# binutils `as` instead of LCCC.  Kbuild drives .S files through $(CC) anyway,
# but AS is consumed by a handful of arch rules and must be consistent.
exec make ARCH=x86_64 CC="$LCCC" LD="$LCCC_LD" AS="$LCCC" -j"$JOBS" "${targets[@]}"
