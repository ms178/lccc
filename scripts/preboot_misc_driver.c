#define _GNU_SOURCE
#include <setjmp.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <fcntl.h>
#include <unistd.h>
#include <sys/stat.h>
#include <sys/mman.h>

extern unsigned long decompress_kernel(unsigned char *outbuf, unsigned long virt_addr,
				       void (*error)(char *x));

unsigned char input_data[8 * 1024 * 1024];
unsigned int input_len, output_len;

void accept_memory(void) {}
void choose_random_location(void) {}
void cleanup_exception_handling(void) {}
int cmdline_find_option_bool(const char *o) { (void)o; return 0; }
void early_tdx_detect(void) {}
unsigned long get_rsdp_addr(void) { return 0; }
int init_unaccepted_memory(void) { return 0; }
unsigned long sev_status;

static jmp_buf jb;
static char last_err[256];
void error(char *x)
{
	snprintf(last_err, sizeof last_err, "%s", x ? x : "(null)");
	longjmp(jb, 1);
}

static unsigned char outbuf[20 * 1024 * 1024];
static unsigned char zscratch[6 * 1024 * 1024];
static unsigned char plainbuf[20 * 1024 * 1024];

static int run(const char *tag, unsigned char *z, unsigned zlen,
	       unsigned char *plain, unsigned plen, int check_plain)
{
	if (zlen > sizeof input_data || plen > sizeof outbuf) {
		fprintf(stderr, "%s too big\n", tag);
		return 1;
	}
	memcpy(input_data, z, zlen);
	input_len = zlen;
	output_len = plen;
	memset(outbuf, 0, plen);
	last_err[0] = 0;
	unsigned long ret = 0;
	if (setjmp(jb) == 0)
		ret = decompress_kernel(outbuf, 0xffffffff81000000UL, error);
	int fail = 0;
	if (last_err[0]) {
		if (strstr(last_err, "not a valid ELF")) {
			if (check_plain && memcmp(outbuf, plain, plen) != 0) {
				printf("%s DECOMPRESS-OK-but-MISMATCH\n", tag);
				fail = 1;
			} else {
				printf("%s DECOMPRESS-OK (parse_elf)\n", tag);
			}
		} else {
			printf("%s FAIL error=%s ret=%lx\n", tag, last_err, ret);
			fail = 1;
		}
	} else if (ret == ~0UL) {
		printf("%s FAIL ULONG_MAX\n", tag);
		fail = 1;
	} else {
		printf("%s returned entry=%lx\n", tag, ret);
	}
	return fail;
}

/* Host libc I/O: do not call malloc (hijacked by misc.c's bump allocator). */
static unsigned read_path(const char *path, unsigned char *dst, unsigned cap)
{
	int fd = open(path, O_RDONLY);
	if (fd < 0) { perror(path); exit(2); }
	unsigned n = 0;
	for (;;) {
		ssize_t r = read(fd, dst + n, cap - n);
		if (r < 0) { perror("read"); exit(2); }
		if (r == 0) break;
		n += (unsigned)r;
		if (n == cap) break;
	}
	close(fd);
	return n;
}

int main(int argc, char **argv)
{
	int fails = 0;
	int sizes[] = {256, 320, 1024, 4096, 65536};
	for (int si = 0; si < 5; si++) {
		int n = sizes[si];
		for (int i = 0; i < n; i++) plainbuf[i] = (unsigned char)(i & 255);
		char zpath[80];
		snprintf(zpath, sizeof zpath, "/tmp/zstd-oracle/p%d.zst", n);
		/* Use already-created files from previous run if present. */
		if (access(zpath, R_OK) != 0) {
			char cmd[256];
			snprintf(cmd, sizeof cmd,
				 "python3 -c \"open('/tmp/t','wb').write(bytes(i&255 for i in range(%d)))\" && zstd -q -f -6 --ultra -o %s /tmp/t",
				 n, zpath);
			if (system(cmd) != 0) { fprintf(stderr, "zstd failed\n"); return 2; }
		}
		unsigned zlen = read_path(zpath, zscratch, sizeof zscratch);
		char tag[80];
		snprintf(tag, sizeof tag, "pattern-%d", n);
		fails += run(tag, zscratch, zlen, plainbuf, n, 1);
		memcpy(zscratch + zlen, &n, 4); /* LE size on little-endian host */
		snprintf(tag, sizeof tag, "pattern-%d-trailer4", n);
		fails += run(tag, zscratch, zlen + 4, plainbuf, n, 1);
	}
	if (argc >= 3) {
		unsigned zlen = read_path(argv[1], zscratch, sizeof zscratch);
		unsigned plen = read_path(argv[2], plainbuf, sizeof plainbuf);
		fails += run("piggy", zscratch, zlen, plainbuf, plen, 1);
	}
	return fails ? 1 : 0;
}
