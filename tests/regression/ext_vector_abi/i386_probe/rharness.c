#include <stdio.h>
#include <string.h>
extern unsigned char sinkr[]; extern const unsigned sink_size, lane_size; void call_get(void);
unsigned char xtag[64] __attribute__((aligned(16)));
const double one = 1.0, two = 2.0;
__asm__(".text\n.globl getx\ngetx:\n"
" movdqa xtag,%xmm0\n movdqa xtag+16,%xmm1\n movdqa xtag+32,%xmm2\n movdqa xtag+48,%xmm3\n"
" fldl two\n fldl one\n mov $0x63626160,%eax\n mov $0x67666564,%edx\n mov $0x6b6a6968,%ecx\n ret\n");
int main(void) {
  unsigned i, k; for (i = 0; i < 64; i++) xtag[i] = 0x10 + i;
  memset(sinkr, 0, sink_size); call_get();
  printf("RETc:");
  for (i = 0; i < sink_size; i += lane_size) {
    unsigned char b = sinkr[i]; const char *n = 0; char buf[24];
    float f; double d; memcpy(&f, sinkr + i, 4); memcpy(&d, sinkr + i, 8);
    if ((lane_size == 4 && f == 1.0f) || (lane_size == 8 && d == 1.0)) n = "ST0";
    else if ((lane_size == 4 && f == 2.0f) || (lane_size == 8 && d == 2.0)) n = "ST1";
    else if (b >= 0x10 && b < 0x50) { snprintf(buf, sizeof buf, "X%u.%u", (b - 0x10) / 16, (b - 0x10) % 16); n = buf; }
    else if (b >= 0x60 && b < 0x6c) { static const char *g[] = {"EAX", "EDX", "ECX"}; snprintf(buf, sizeof buf, "%s.%u", g[(b - 0x60) / 4], (b - 0x60) % 4); n = buf; }
    else { snprintf(buf, sizeof buf, "?%02x", b); n = buf; }
    printf(" %s", n);
  }
  printf("\n"); (void)k; return 0;
}
