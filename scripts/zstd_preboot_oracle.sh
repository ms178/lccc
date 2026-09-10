#!/usr/bin/env bash
# Userspace oracle of the preboot ZSTD path (misc.c's decompress_kernel call).
# Compiles a TU that #includes decompress_unzstd.c with STATIC+MALLOC_VISIBLE
# like arch/x86/boot/compressed/misc.c, then calls
#   __decompress(input_data, input_len, NULL, NULL, out, output_len, NULL, error)
# where input_len/output_len are extern unsigned int (the piggy.S types).
#
# Two decisive properties (both learned the hard way, session 12):
#   * BMI2 is MASKED by default (ZSTD_cpuid_bmi2 -> 0) so the `_default`
#     clones that the qemu64 guest actually executes are the ones under test;
#     EXTRA_CFLAGS=-DZSTD_ORACLE_HOST_CPU restores host CPUID dispatch.
#   * Besides the separate-buffer driver, an IN-PLACE driver reproduces the
#     guest geometry (input copied to out+in_off, decompress into out).
# Leaves $OUT/{oracle.c,driver.c,driver_inplace.c,cflags.rsp} behind for
# scripts/zstd_oracle_cc.sh.
set -euo pipefail

K=${KERNEL_DIR:-/home/user/kernel-work/linux-6.18.50}
LCCC=${LCCC:-/home/user/lccc/target/fastbuild/lccc}
OUT=${OUT:-/tmp/zstd-oracle}
mkdir -p "$OUT"

# Compressed-boot flags (arch/x86/boot/compressed/Makefile), plus userspace
# linkability. Do NOT add -mcmodel=kernel. Do NOT pass -fno-builtin memset
# (that was a harness SEGV).
CFLAGS_COMMON="-m64 -O2 -std=gnu18 -fno-strict-aliasing -fPIE -fno-jump-tables \
  -mcmodel=small -mno-red-zone -mno-mmx -mno-sse -ffreestanding \
  -fno-stack-protector -fno-asynchronous-unwind-tables -fshort-wchar \
  -Wno-pointer-sign -Wno-address-of-packed-member"

# KBUILD_MODNAME/MODFILE must be C strings. Keep them in a response file
# so unquoted expansion cannot eat the quotes.
cat > "$OUT/cflags.rsp" <<RSP
-nostdinc
-I$K/arch/x86/boot/compressed
-I$K/arch/x86/include
-I$K/arch/x86/include/generated
-I$K/include
-I$K/include/generated
-I$K/include/uapi
-I$K/arch/x86/include/uapi
-I$K/arch/x86/include/generated/uapi
-I$K/include/generated/uapi
-I$K/lib
-include
$K/include/linux/compiler-version.h
-include
$K/include/linux/kconfig.h
-include
$K/include/linux/compiler_types.h
-include
$K/include/linux/hidden.h
-D__KERNEL__
-DDISABLE_BRANCH_PROFILING
-D__DISABLE_EXPORTS
-DKBUILD_BASENAME='"oracle"'
-DKBUILD_MODNAME='"oracle"'
-DKBUILD_MODFILE='"arch/x86/boot/compressed/oracle"'
RSP

# --- oracle TU: same include shape as misc.c, minus extract_kernel ----------
cat > "$OUT/oracle.c" <<'EOF'
/* Mirror misc.c's prologue exactly: misc.h provides memptr, the extern
 * free_mem_ptr declarations and (via its include chain) size_t; the boot
 * string.h provides the memset/memcpy/memmove declarations. */
#include "misc.h"
#include "error.h"
#include "../string.h"

#define STATIC		static
/* misc.c defines MALLOC_VISIBLE empty (multi-TU boot image). In the
 * single-TU oracle that exports the boot malloc over libc for the whole
 * program — the driver therefore must NOT use libc malloc/calloc (a
 * first-calls-segfault harness bug when free_mem_ptr is still 0). The
 * constructor below arms the boot heap before main, and the driver
 * allocates via mmap so the multi-megabyte piggy buffers never touch the
 * 192 KiB boot heap. */
#define MALLOC_VISIBLE
#include <linux/decompress/mm.h>
#define memzero(s, n)	memset((s), 0, (n))
#ifndef memmove
#define memmove memmove
void *memmove(void *dest, const void *src, size_t n);
#endif
void *memset(void *s, int c, size_t n);
void *memcpy(void *dest, const void *src, size_t n);

memptr free_mem_ptr;
memptr free_mem_end_ptr;

/* linux/hidden.h (force-included) marks the boot string decls hidden, so
 * userspace libc cannot satisfy them; provide freestanding implementations
 * in the TU itself, exactly like arch/x86/boot/string.c does for the boot
 * image. */
void *memset(void *s, int c, size_t n)
{
	unsigned char *p = s;
	while (n--)
		*p++ = (unsigned char)c;
	return s;
}
void *memcpy(void *dest, const void *src, size_t n)
{
	unsigned char *d = dest;
	const unsigned char *s = src;
	while (n--)
		*d++ = *s++;
	return dest;
}
void *memmove(void *dest, const void *src, size_t n)
{
	unsigned char *d = dest;
	const unsigned char *s = src;
	if (d < s) {
		while (n--)
			*d++ = *s++;
	} else {
		d += n; s += n;
		while (n--)
			*--d = *--s;
	}
	return dest;
}

/* CPU-feature MASK (default ON): the qemu64 guest reports no BMI2, so the
 * boot decompressor runs the `_default` (non-BMI2) clones of every
 * HUF/ZSTD hot loop.  A host with BMI2 would silently exercise the `_bmi2`
 * clones instead and MATCH while the guest fails (session 12: the -O1/-O2
 * miscompiles lived only in HUF_decompress4X2_usingDTable_internal_default
 * and ZSTD_decompressSequences_body).  Build with -DZSTD_ORACLE_HOST_CPU to
 * follow the host CPUID instead. */
#ifndef ZSTD_ORACLE_HOST_CPU
#include "zstd/common/cpu.h"
static inline int zstd_oracle_no_bmi2(ZSTD_cpuid_t c) { (void)c; return 0; }
#define ZSTD_cpuid_bmi2 zstd_oracle_no_bmi2
#endif

#include "DECOMPRESS_UNZSTD_PATH_PLACEHOLDER"

/* misc.h -> asm/boot.h already defines BOOT_HEAP_SIZE for the boot heap
 * geometry; the oracle heap is a separate, larger userspace allocation. */
static unsigned char boot_heap[0x30000] __attribute__((aligned(8)));

/* Arm the boot heap before main: the exported boot malloc must never be
 * called with free_mem_ptr == 0 by the driver's own allocations. */
__attribute__((constructor)) static void oracle_boot_heap_init(void)
{
	free_mem_ptr = (unsigned long)boot_heap;
	free_mem_end_ptr = free_mem_ptr + sizeof(boot_heap);
}

/* The driver defines POINTER globals (mmap'd payloads). Declaring them
 * as arrays here reads the pointer's own bytes as compressed data —
 * the classic wrong-magic harness bug. piggy.S in the real boot defines
 * actual arrays; the oracle passes the pointer through, which exercises
 * the identical __decompress code path. */
extern unsigned char *input_data;
extern unsigned int input_len, output_len;

int preboot_decompress(unsigned char *outbuf, void (*error)(char *x))
{
	free_mem_ptr = (unsigned long)boot_heap;
	free_mem_end_ptr = free_mem_ptr + sizeof(boot_heap);
	return __decompress(input_data, input_len, 0, 0, outbuf, output_len,
			    0, error);
}

int preboot_decompress_sized(unsigned char *in, unsigned int in_len,
			     unsigned char *out, unsigned int out_len,
			     void (*error)(char *x))
{
	free_mem_ptr = (unsigned long)boot_heap;
	free_mem_end_ptr = free_mem_ptr + sizeof(boot_heap);
	return __decompress(in, in_len, 0, 0, out, out_len, 0, error);
}
EOF

# --- driver -----------------------------------------------------------------
cat > "$OUT/driver.c" <<'EOF'
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <unistd.h>
#include <fcntl.h>
#include <sys/stat.h>
#include <sys/mman.h>
#include <sys/wait.h>

extern int preboot_decompress(unsigned char *outbuf, void (*error)(char *x));
extern int preboot_decompress_sized(unsigned char *in, unsigned int in_len,
				    unsigned char *out, unsigned int out_len,
				    void (*error)(char *x));

unsigned char *input_data;
unsigned int input_len, output_len;

/* The oracle TU exports the BOOT allocator as the program-wide malloc
 * (MALLOC_VISIBLE empty, misc.h compatibility). All driver allocations
 * must therefore bypass malloc entirely: mmap gives page-granular,
 * zero-initialized memory that no boot-heap accounting touches. */
static void *xalloc(size_t n)
{
	void *p = mmap(NULL, n, PROT_READ | PROT_WRITE,
		       MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
	if (p == MAP_FAILED) { perror("mmap"); exit(2); }
	return p;
}

static char last_err[256];
static void on_error(char *x)
{
	snprintf(last_err, sizeof last_err, "%s", x ? x : "(null)");
	fprintf(stderr, "error: %s\n", last_err);
}

/* The boot image runs __decompress exactly once; the boot allocator's
 * STATIC state (malloc_ptr/malloc_count in linux/decompress/mm.h) is not
 * resettable from outside, and a crash in one case must not take the
 * whole oracle down. Run every case in a fresh fork(): each test sees
 * virgin allocator state — single-shot semantics identical to the boot. */
static int run_one(const char *tag, unsigned char *in, unsigned in_len,
		   unsigned char *expect, unsigned expect_len, int via_extern)
{
	pid_t pid = fork();
	if (pid == 0) {
		unsigned char *out = xalloc(expect_len + 64);
		int rc;
		last_err[0] = 0;
		if (via_extern) {
			input_data = in;
			input_len = in_len;
			output_len = expect_len;
			rc = preboot_decompress(out, on_error);
		} else {
			rc = preboot_decompress_sized(in, in_len, out, expect_len, on_error);
		}
		if (rc < 0) {
			printf("%s FAIL rc=%d err=%s\n", tag, rc, last_err);
			fflush(stdout);
			_exit(1);
		}
		if (memcmp(out, expect, expect_len) != 0) {
			printf("%s FAIL mismatch\n", tag);
			fflush(stdout);
			_exit(1);
		}
		printf("%s MATCH %u -> %u\n", tag, in_len, expect_len);
		fflush(stdout);
		_exit(0);
	}
	int st = 0;
	waitpid(pid, &st, 0);
	if (WIFSIGNALED(st)) {
		printf("%s FAIL signal=%d\n", tag, WTERMSIG(st));
		return 1;
	}
	return WIFEXITED(st) && WEXITSTATUS(st) == 0 ? 0 : 1;
}

static unsigned char *read_all(const char *path, unsigned *n)
{
	int fd = open(path, O_RDONLY);
	if (fd < 0) { perror(path); exit(2); }
	struct stat st;
	fstat(fd, &st);
	unsigned char *p = xalloc(st.st_size);
	if (read(fd, p, st.st_size) != st.st_size) { perror("read"); exit(2); }
	close(fd);
	*n = (unsigned)st.st_size;
	return p;
}

int main(int argc, char **argv)
{
	int fails = 0;
	/* Patterned 256 / 320 / 4K like the previous oracle table. */
	for (int n = 256; n <= 4096; n = (n == 256 ? 320 : n == 320 ? 1024 : n == 1024 ? 4096 : 8192)) {
		unsigned char *plain = xalloc(n);
		for (int i = 0; i < n; i++) plain[i] = (unsigned char)(i & 255);
		char zpath[64];
		snprintf(zpath, sizeof zpath, "/tmp/zstd-oracle/p%d.zst", n);
		char cmd[256];
		snprintf(cmd, sizeof cmd, "zstd -q -f -6 --ultra -o %s", zpath);
		FILE *w = popen(cmd, "w");
		fwrite(plain, 1, n, w);
		pclose(w);
		unsigned zlen;
		unsigned char *z = read_all(zpath, &zlen);
		char tag[64];
		snprintf(tag, sizeof tag, "pattern-%d-args", n);
		fails += run_one(tag, z, zlen, plain, n, 0);
		snprintf(tag, sizeof tag, "pattern-%d-extern", n);
		fails += run_one(tag, z, zlen, plain, n, 1);
		/* 4-byte LE size trailer like size_append */
		unsigned char *zt = xalloc(zlen + 4);
		memcpy(zt, z, zlen);
		zt[zlen] = n & 255; zt[zlen+1] = (n >> 8) & 255;
		zt[zlen+2] = (n >> 16) & 255; zt[zlen+3] = (n >> 24) & 255;
		snprintf(tag, sizeof tag, "pattern-%d-trailer-extern", n);
		fails += run_one(tag, zt, zlen + 4, plain, n, 1);
		munmap(plain, n); munmap(z, zlen); munmap(zt, zlen + 4);
	}

	if (argc >= 3) {
		unsigned zlen, plen;
		unsigned char *z = read_all(argv[1], &zlen);
		unsigned char *p = read_all(argv[2], &plen);
		fails += run_one("piggy-args", z, zlen, p, plen, 0);
		fails += run_one("piggy-extern", z, zlen, p, plen, 1);
		munmap(z, zlen); munmap(p, plen);
	}
	return fails ? 1 : 0;
}
EOF

# --- in-place driver: the guest geometry --------------------------------
# head_64.S relocates the ZO to the END of the output buffer and misc.c then
# decompresses in place: out=B, in=B+in_off with in_off chosen so the input
# tail is only overwritten after it has been consumed (see
# arch/x86/boot/header.S z_extract_offset / init_size).  The separate-buffer
# driver above can MATCH while this shape fails, because a miscompiled
# sequence decoder that reads *stale* literals only shows up when the output
# window overlaps the not-yet-consumed input.  Default in_off is the value
# observed for the VM-config kernel (0xc0029b); pass another as argv[3].
cat > "$OUT/driver_inplace.c" <<'EOF'
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <fcntl.h>
#include <unistd.h>
#include <sys/mman.h>
#include <sys/stat.h>
extern int preboot_decompress_sized(unsigned char *in, unsigned in_len,
				    unsigned char *out, unsigned out_len,
				    void (*error)(char *));
unsigned char *input_data; unsigned input_len, output_len;
static void on_error(char *x) { fprintf(stderr, "error: %s\n", x); }
static unsigned char *rd(const char *p, unsigned *n)
{
	int fd = open(p, O_RDONLY);
	struct stat st;
	if (fd < 0 || fstat(fd, &st) < 0) { perror(p); exit(2); }
	unsigned char *b = mmap(0, st.st_size, PROT_READ | PROT_WRITE,
				MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
	if (read(fd, b, st.st_size) != st.st_size) { perror(p); exit(2); }
	close(fd);
	*n = st.st_size;
	return b;
}
int main(int argc, char **argv)
{
	if (argc < 3) {
		fprintf(stderr, "usage: %s vmlinux.bin.zst vmlinux.bin [in_off] [dump]\n", argv[0]);
		return 2;
	}
	unsigned zl, pl;
	unsigned char *z = rd(argv[1], &zl), *p = rd(argv[2], &pl);
	unsigned long in_off = argc > 3 ? strtoul(argv[3], 0, 0) : 0xc0029b;
	size_t total = in_off + zl + 0x100000;
	unsigned char *B = mmap(0, total, PROT_READ | PROT_WRITE,
				MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
	memcpy(B + in_off, z, zl);
	int rc = preboot_decompress_sized(B + in_off, zl, B, pl, on_error);
	unsigned first = pl;
	for (unsigned i = 0; i < pl; i++)
		if (B[i] != p[i]) { first = i; break; }
	printf("inplace off=0x%lx rc=%d %s first_diff=0x%x\n", in_off, rc,
	       first == pl ? "MATCH" : "MISMATCH", first);
	if (argc > 4) { FILE *f = fopen(argv[4], "wb"); fwrite(B, 1, pl, f); fclose(f); }
	return first == pl && rc >= 0 ? 0 : 1;
}
EOF

# The TU heredoc above is quoted (no expansion), so patch the kernel-tree
# absolute include path in afterwards. KERNEL_DIR (K) is the single source
# of truth — no hardcoded /home/user.
sed -i "s|DECOMPRESS_UNZSTD_PATH_PLACEHOLDER|$K/lib/decompress_unzstd.c|" \
    "$OUT/oracle.c"

compile_one() {
  local cc=$1 name=$2
  echo "CC  oracle ($name)"
  $cc $CFLAGS_COMMON ${EXTRA_CFLAGS:-} @"$OUT/cflags.rsp" -c "$OUT/oracle.c" -o "$OUT/oracle-$name.o"
  echo "CC  driver ($name)"
  # driver is userspace; use host gcc always for libc
  gcc -O2 -fPIE -c "$OUT/driver.c" -o "$OUT/driver.o"
  gcc -pie -o "$OUT/oracle-$name" "$OUT/driver.o" "$OUT/oracle-$name.o"
  gcc -O2 -fPIE -c "$OUT/driver_inplace.c" -o "$OUT/driver_inplace.o"
  gcc -pie -o "$OUT/oracle-inplace-$name" "$OUT/driver_inplace.o" "$OUT/oracle-$name.o"
}

compile_one gcc gcc
compile_one "$LCCC" lccc

echo "==== gcc ===="
"$OUT/oracle-gcc" "$@" || true
echo "==== lccc ===="
"$OUT/oracle-lccc" "$@" || true

# In-place (guest geometry) run on the real piggy payload, when present.
ZFILE=${ZFILE:-$K/arch/x86/boot/compressed/vmlinux.bin.zst}
PFILE=${PFILE:-$K/arch/x86/boot/compressed/vmlinux.bin}
if [[ -f "$ZFILE" && -f "$PFILE" ]]; then
  echo "==== in-place (guest geometry) ===="
  printf '%-6s ' gcc;  "$OUT/oracle-inplace-gcc"  "$ZFILE" "$PFILE" || true
  printf '%-6s ' lccc; "$OUT/oracle-inplace-lccc" "$ZFILE" "$PFILE" || true
fi
