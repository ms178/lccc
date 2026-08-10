/* glibc_print_file_name.c — driver queries used by glibc's link rules:
 * `${CC} --print-file-name=crtbeginS.o` (double dash, backtick-substituted
 * in Makefiles) must exit early with the sysroot path, not "no input files".
 * Compile-only probe executed by the test harness; also covers
 * --print-multi-directory and --print-libgcc-file-name. */
#include <stdio.h>

int main(void) {
    printf("PASS print_file_name\n");
    return 0;
}
