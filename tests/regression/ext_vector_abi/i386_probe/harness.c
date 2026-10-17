/* i386 by-value convention oracle: which bytes of which location carry each
   byte of an argument / return value, for a Clang-compiled callee. */
#include <stdio.h>
#include <string.h>
extern unsigned char sink[]; extern const unsigned sink_size, lane_size;
extern void put(void); extern void get(void);
unsigned char stk[256] __attribute__((aligned(16)));
unsigned char xin[48] __attribute__((aligned(16)));
unsigned char retbuf[256] __attribute__((aligned(16)));
unsigned out_eax, out_edx, out_ecx, sw_before, sw_after, esp_delta;
unsigned char out_xmm[64] __attribute__((aligned(16)));
unsigned char out_st[3][10];
void tramp(void (*fn)(void), unsigned nstk, int pass_sret);
__asm__(
".text\n.globl tramp\ntramp:\n"
" push %ebp\n mov %esp,%ebp\n push %esi\n push %edi\n push %ebx\n"
" fnstsw %ax\n movzwl %ax,%eax\n mov %eax,sw_before\n"
" mov 12(%ebp),%ecx\n sub %ecx,%esp\n and $-16,%esp\n"
" mov %esp,%edi\n lea stk,%esi\n rep movsb\n"
" cmpl $0,16(%ebp)\n je 1f\n sub $12,%esp\n lea retbuf,%eax\n push %eax\n1:\n"
" movdqa xin,%xmm0\n movdqa xin+16,%xmm1\n movdqa xin+32,%xmm2\n"
" mov $0x43424140,%ecx\n mov $0x47464544,%edx\n"
" mov %esp,%ebx\n call *8(%ebp)\n sub %esp,%ebx\n mov %ebx,esp_delta\n"
" mov %eax,out_eax\n mov %edx,out_edx\n mov %ecx,out_ecx\n"
" movdqu %xmm0,out_xmm\n movdqu %xmm1,out_xmm+16\n movdqu %xmm2,out_xmm+32\n movdqu %xmm3,out_xmm+48\n"
" fnstsw %ax\n movzwl %ax,%eax\n mov %eax,sw_after\n"
" lea -12(%ebp),%esp\n pop %ebx\n pop %edi\n pop %esi\n pop %ebp\n ret\n");
static const char *src(unsigned char b) {
  static char buf[32];
  if (b >= 0x80) snprintf(buf, sizeof buf, "S%u", b - 0x80);
  else if (b >= 0x10 && b < 0x40) snprintf(buf, sizeof buf, "X%u.%u", (b - 0x10) / 16, (b - 0x10) % 16);
  else if (b >= 0x40 && b < 0x44) snprintf(buf, sizeof buf, "ECX.%u", b - 0x40);
  else if (b >= 0x44 && b < 0x48) snprintf(buf, sizeof buf, "EDX.%u", b - 0x44);
  else snprintf(buf, sizeof buf, "?%02x", b);
  return buf;
}
int main(void) {
  unsigned n = sink_size, i;
  for (i = 0; i < sizeof stk; i++) stk[i] = 0x80 | (i & 0x7f);
  for (i = 0; i < sizeof xin; i++) xin[i] = 0x10 + i;
  memset(sink, 0, n);
  tramp(put, 128, 0);
  printf("ARG:");
  for (i = 0; i < n; i += lane_size) printf(" %s", src(sink[i]));
  /* contiguity check inside each lane */
  for (i = 0; i < n; i++) if (i % lane_size && sink[i] != (unsigned char)(sink[i - 1] + 1)) { printf(" (noncontig@%u)", i); break; }
  printf("\nRET:");
  for (i = 0; i < n; i++) sink[i] = 0x80 | (i & 0x7f);
  /* probe once without sret pointer to detect register returns safely is
     impossible if the callee needs one; always pass one. */
  memset(retbuf, 0, sizeof retbuf);
  tramp(get, 0, 1);
  int depth = (int)((sw_before >> 11) & 7) - (int)((sw_after >> 11) & 7);
  if (depth < 0) depth += 8;
  unsigned char st[3][16]; memset(st, 0, sizeof st);
  for (int k = 0; k < depth && k < 3; k++) {
    if (lane_size == 4) __asm__ volatile("fstps %0" : "=m"(*(float *)st[k]));
    else if (lane_size == 8) __asm__ volatile("fstpl %0" : "=m"(*(double *)st[k]));
    else __asm__ volatile("fstpt %0" : "=m"(*(long double *)st[k]));
  }
  int sret = 0;
  for (i = 0; i < n; i++) if (retbuf[i]) sret = 1;
  unsigned char e[12]; memcpy(e, &out_eax, 4); memcpy(e + 4, &out_edx, 4); memcpy(e + 8, &out_ecx, 4);
  static const char *gp[] = {"EAX", "EDX", "ECX"};
  for (i = 0; i < n; i += lane_size) {
    unsigned char want = 0x80 | (i & 0x7f); int found = 0; unsigned k;
    if (sret) { for (k = 0; k < n; k++) if (retbuf[k] == want) { printf(" M%u", k); found = 1; break; } }
    else {
      for (k = 0; k < 12 && !found; k++) if (e[k] == want) { printf(" %s.%u", gp[k / 4], k % 4); found = 1; }
      for (k = 0; k < 64 && !found; k++) if (out_xmm[k] == want) { printf(" X%u.%u", k / 16, k % 16); found = 1; }
      for (k = 0; k < 3 * 16 && !found && (int)(k / 16) < depth; k++) if (st[k / 16][k % 16] == want) { printf(" ST%u.%u", k / 16, k % 16); found = 1; }
    }
    if (!found) printf(" -");
  }
  if (sret) printf(" [sret pop=%d]", -(int)esp_delta);
  else if (depth) printf(" [x87 %d]", depth);
  printf("\n");
  return 0;
}
