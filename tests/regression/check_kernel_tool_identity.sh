#!/usr/bin/env bash
# Red-team the Kbuild tool-identity invalidation policy without building Linux.
set -euo pipefail
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
ORACLE="$ROOT/scripts/kernel_tool_identity.sh"
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
K="$T/kernel"
B="$T/bin"
mkdir -p "$K/arch/x86/entry/vdso" "$K/arch/x86/boot/compressed" "$B"
printf compiler-v1 > "$T/lccc"
printf linker-v1 > "$T/lccc-ld"

# Fake Kbuild clean: record invocation and model products from several classes.
cat > "$B/make" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$KERNEL_TEST_LOG"
[[ " $* " == *' clean '* ]] || exit 91
rm -f target.o generated.s archive.a arch/x86/boot/bzImage vmlinux
EOF
chmod +x "$B/make" "$ORACLE"
export PATH="$B:$PATH" KERNEL_TEST_LOG="$T/make.log"

seed_products() {
  : > "$K/target.o"
  : > "$K/generated.s"
  : > "$K/archive.a"
  : > "$K/arch/x86/entry/vdso/vdso64.so.dbg"
  : > "$K/arch/x86/boot/compressed/vmlinux.bin"
  : > "$K/arch/x86/boot/bzImage"
  : > "$K/vmlinux"
  # Link products in the compressed directory, plus the CHECKED-IN source that
  # shares their `vmlinux*` prefix. A bare glob purge deletes the source too,
  # and the loss does not fail there: it fails the NEXT build with
  #   No rule to make target 'arch/x86/boot/compressed/vmlinux.lds'
  # after every translation unit has already been rebuilt.
  : > "$K/arch/x86/boot/compressed/vmlinux"
  : > "$K/arch/x86/boot/compressed/vmlinux.lds"
  printf '/* checked-in linker-script source, not a product */\n' \
      > "$K/arch/x86/boot/compressed/vmlinux.lds.S"
}

# First observation establishes identity and must preserve existing products.
seed_products
"$ORACLE" "$K" "$T/lccc" "$T/lccc-ld"
[[ -f $K/target.o && -f $K/vmlinux && ! -e $T/make.log ]]

# Identical tools are a strict no-op.
"$ORACLE" "$K" "$T/lccc" "$T/lccc-ld"
[[ ! -e $T/make.log ]]

# Upgrading a tree that has only the legacy combined stamp must conservatively
# clean once: its old value cannot identify whether CC or LD changed.
rm -f "$K/.lccc-compiler-stamp" "$K/.lccc-linker-stamp" "$T/make.log"
: > "$K/.lccc-tools-stamp"
seed_products
"$ORACLE" "$K" "$T/lccc" "$T/lccc-ld"
[[ $(wc -l < "$T/make.log") -eq 1 ]]
grep -qx 'ARCH=x86_64 clean' "$T/make.log"
[[ ! -e $K/target.o && ! -e $K/generated.s && ! -e $K/archive.a ]]
[[ ! -e $K/.lccc-tools-stamp ]]

# Linker-only change preserves compilation products but invalidates every link
# class, including wildcarded compressed/vDSO intermediates.
seed_products
rm -f "$T/make.log"
printf linker-v2 > "$T/lccc-ld"
"$ORACLE" "$K" "$T/lccc" "$T/lccc-ld"
[[ -f $K/target.o && -f $K/generated.s && -f $K/archive.a ]]
[[ ! -e $K/vmlinux && ! -e $K/arch/x86/boot/bzImage ]]
[[ ! -e $K/arch/x86/entry/vdso/vdso64.so.dbg ]]
[[ ! -e $K/arch/x86/boot/compressed/vmlinux.bin ]]
[[ ! -e $K/arch/x86/boot/compressed/vmlinux ]]
[[ ! -e $K/arch/x86/boot/compressed/vmlinux.lds ]]
# The purge is product-only. The checked-in linker-script source shares the
# `vmlinux*` prefix and must survive it, or the next build has no rule to
# regenerate `vmlinux.lds` from.
[[ -f $K/arch/x86/boot/compressed/vmlinux.lds.S ]]
[[ ! -e $T/make.log ]]

# Compiler-only change invokes the authoritative clean, not the weaker linker
# purge. This pins the if/elif asymmetry independently of a simultaneous edit.
seed_products
rm -f "$T/make.log"
printf compiler-v2 > "$T/lccc"
out=$("$ORACLE" "$K" "$T/lccc" "$T/lccc-ld")
[[ $(wc -l < "$T/make.log") -eq 1 ]]
grep -qx 'ARCH=x86_64 clean' "$T/make.log"
[[ $out != *'purging link-only products'* ]]
[[ ! -e $K/target.o && ! -e $K/generated.s && ! -e $K/archive.a ]]

# Changing both tools simultaneously must still take the stronger compiler
# path, never merely the linker purge.
seed_products
rm -f "$T/make.log"
printf compiler-v3 > "$T/lccc"
printf linker-v3 > "$T/lccc-ld"
"$ORACLE" "$K" "$T/lccc" "$T/lccc-ld"
[[ $(wc -l < "$T/make.log") -eq 1 ]]
grep -qx 'ARCH=x86_64 clean' "$T/make.log"
[[ ! -e $K/target.o && ! -e $K/generated.s && ! -e $K/archive.a ]]

# Published stamps are full SHA-256 values, and the obsolete unsafe combined
# stamp is actively retired.
[[ $(wc -c < "$K/.lccc-compiler-stamp") -eq 65 ]]
[[ $(wc -c < "$K/.lccc-linker-stamp") -eq 65 ]]
: > "$K/.lccc-tools-stamp"
"$ORACLE" "$K" "$T/lccc" "$T/lccc-ld"
[[ ! -e $K/.lccc-tools-stamp ]]

# Relative tool arguments are resolved before the script enters the kernel
# directory; validation and hashing therefore use the same files.
rm -f "$T/make.log"
(cd "$T" && "$ORACLE" "$K" lccc lccc-ld)
[[ ! -e $T/make.log ]]

echo 'kernel tool identity invalidation: PASS'
