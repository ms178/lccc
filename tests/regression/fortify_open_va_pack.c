#define _FORTIFY_SOURCE 3
#include <fcntl.h>
#include <unistd.h>
__attribute__((noinline)) int wrap(const char *p, int flags, int mode) { return open(p,flags,mode); }
int main(void) { int fd=wrap("/dev/null",O_RDONLY,0); if (fd<0) return 1; return close(fd); }
