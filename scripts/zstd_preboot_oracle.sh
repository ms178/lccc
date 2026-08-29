#!/usr/bin/env bash
# Userspace oracle of the preboot ZSTD path (misc.c's decompress_kernel call).
# Compiles a TU that #includes decompress_unzstd.c with STATIC+MALLOC_VISIBLE
# like arch/x86/boot/compressed/misc.c, then calls
#   __decompress(input_data, input_len, NULL, NULL, out, output_len, NULL, error)
# where input_len/output_len are extern unsigned int (the piggy.S types).
set -euo pipefail

K=${KERNEL_DIR:-/home/user/kernel-work/linux-6.18.47}
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
-D_SETUP
-DDISABLE_BRANCH_PROFILING
-D__DISABLE_EXPORTS
-DKBUILD_BASENAME='"oracle"'
-DKBUILD_MODNAME='"oracle"'
-DKBUILD_MODFILE='"arch/x86/boot/compressed/oracle"'
RSP

# --- oracle TU: same include shape as misc.c, minus extract_kernel ----------
cat > "$OUT/oracle.c" <<'EOF'
#define STATIC		static
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

#include "/home/user/kernel-work/linux-6.18.47/lib/decompress_unzstd.c"

enum { BOOT_HEAP_SIZE = 0x30000 };
static unsigned char boot_heap[BOOT_HEAP_SIZE] __attribute__((aligned(8)));

extern unsigned char input_data[];
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

extern int preboot_decompress(unsigned char *outbuf, void (*error)(char *x));
extern int preboot_decompress_sized(unsigned char *in, unsigned int in_len,
				    unsigned char *out, unsigned int out_len,
				    void (*error)(char *x));

unsigned char *input_data;
unsigned int input_len, output_len;

static char last_err[256];
static void on_error(char *x)
{
	snprintf(last_err, sizeof last_err, "%s", x ? x : "(null)");
	fprintf(stderr, "error: %s\n", last_err);
}

static int run_one(const char *tag, unsigned char *in, unsigned in_len,
		   unsigned char *expect, unsigned expect_len, int via_extern)
{
	unsigned char *out = calloc(1, expect_len + 64);
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
		free(out);
		return 1;
	}
	if (memcmp(out, expect, expect_len) != 0) {
		printf("%s FAIL mismatch\n", tag);
		free(out);
		return 1;
	}
	printf("%s MATCH %u -> %u\n", tag, in_len, expect_len);
	free(out);
	return 0;
}

static unsigned char *read_all(const char *path, unsigned *n)
{
	int fd = open(path, O_RDONLY);
	if (fd < 0) { perror(path); exit(2); }
	struct stat st;
	fstat(fd, &st);
	unsigned char *p = malloc(st.st_size);
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
		unsigned char *plain = malloc(n);
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
		unsigned char *zt = malloc(zlen + 4);
		memcpy(zt, z, zlen);
		zt[zlen] = n & 255; zt[zlen+1] = (n >> 8) & 255;
		zt[zlen+2] = (n >> 16) & 255; zt[zlen+3] = (n >> 24) & 255;
		snprintf(tag, sizeof tag, "pattern-%d-trailer-extern", n);
		fails += run_one(tag, zt, zlen + 4, plain, n, 1);
		free(plain); free(z); free(zt);
	}

	if (argc >= 3) {
		unsigned zlen, plen;
		unsigned char *z = read_all(argv[1], &zlen);
		unsigned char *p = read_all(argv[2], &plen);
		fails += run_one("piggy-args", z, zlen, p, plen, 0);
		fails += run_one("piggy-extern", z, zlen, p, plen, 1);
		free(z); free(p);
	}
	return fails ? 1 : 0;
}
EOF

compile_one() {
  local cc=$1 name=$2
  echo "CC  oracle ($name)"
  $cc $CFLAGS_COMMON @"$OUT/cflags.rsp" -c "$OUT/oracle.c" -o "$OUT/oracle-$name.o"
  echo "CC  driver ($name)"
  # driver is userspace; use host gcc always for libc
  gcc -O2 -fPIE -c "$OUT/driver.c" -o "$OUT/driver.o"
  gcc -pie -o "$OUT/oracle-$name" "$OUT/driver.o" "$OUT/oracle-$name.o"
}

compile_one gcc gcc
compile_one "$LCCC" lccc

echo "==== gcc ===="
"$OUT/oracle-gcc" || true
echo "==== lccc ===="
"$OUT/oracle-lccc" || true
