/* Two-level expand-and-quote version string (glibc/zstd
 * ZSTD_EXPAND_AND_QUOTE(ZSTD_LIB_VERSION) shape): MAJOR.MINOR.PATCH are
 * separate macros concatenated with '.' in an intermediate macro, then
 * stringified through the two-level # operator chain. The rescan of the
 * replacement text operates on preprocessing tokens: the identifier after
 * the "1" plus "." replacement must still macro-expand (it is NOT part of
 * a pp-number "1.MINOR"). Pre-fix produced "1.MINOR. 0". Both the
 * compile-time _Static_assert and the run-time diff catch it. */
#include <stdio.h>

#define VERSION_MAJOR 1
#define VERSION_MINOR 6
#define VERSION_PATCH 0
#define VER_STRING BASE.VERSION_MINOR.VERSION_PATCH
#define BASE VERSION_MAJOR
#define STR_(x) #x
#define STR(x) STR_(x)

static const char version[] = STR(VER_STRING);

int main(void) {
    _Static_assert(sizeof(version) == 6, "version must stringify as 1.6.0");
    printf("%s\n", version);
    return version[1] == '.' && version[3] == '.' ? 0 : 1;
}
