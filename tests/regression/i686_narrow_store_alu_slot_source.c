/* i686: a NARROW store to a frame slot must not be forwarded into a 32-bit ALU
 * operand that reads the same slot.
 *
 * `forward_slot_loads`' ALU-source arm rewrote
 *     movw %ax, 8(%esp) / addl 8(%esp), %esi   ->   addl %eax, %esi
 * i.e. it substituted the 16-bit store's source register for the FULL 32-bit
 * slot contents, although the slot's upper two bytes still held the value of
 * the earlier `movl`.  Union punning makes that observable: the wide read must
 * see the old high half together with the new low half.
 *
 * Found by the slot-forwarding differential fuzzer (both lccc arms disagreed
 * with gcc, so it predates the CFG forwarder).  Wrong at -O1/-Os/-O2/-O3 on
 * i686; x86-64 was never affected (its store forwarder rewrites loads and
 * requires exact width, or the sound qword->dword case where the stored eight
 * bytes do define the low four).  Reference values below are gcc -m32, which
 * agrees at every optimisation level. */
#include <stdio.h>

union U { unsigned int w; unsigned short h[2]; unsigned char v[4]; };

__attribute__((noinline)) unsigned int word_pun(unsigned k) {
  union U u; unsigned acc = 0;
  u.w = k * 2654435761u;                     /* full-width store defines slot */
  acc += u.w; acc ^= u.v[1] << 8;
  u.h[0] = (unsigned short)(acc & 0xffff);   /* NARROW store: high half lives */
  acc += u.w;                                /* 32-bit read of the whole slot */
  u.v[3] = (unsigned char)k;
  acc += u.w;
  return acc;
}

__attribute__((noinline)) unsigned int byte_pun(unsigned k) {
  union U u; unsigned acc = 0;
  u.w = k * 40503u + 7u;
  u.v[2] = (unsigned char)(k ^ 0x5a);        /* NARROW byte store */
  acc += u.w * 3u;                           /* 32-bit ALU operand on the slot */
  u.v[0] = (unsigned char)acc;
  acc ^= u.w;
  return acc;
}


__attribute__((noinline)) unsigned int cmp_pun(unsigned k) {
  union U u; unsigned acc = 0;
  u.w = k * 2654435761u;
  u.h[1] = (unsigned short)(k * 7u + 3u);    /* NARROW store to the HIGH half */
  if (u.w > 0x80000000u) acc += 1u;          /* cmpl reads all four bytes */
  acc ^= u.w;                                /* xorl reads all four bytes */
  acc += u.w & 0xffffu;                      /* andl reads all four bytes */
  return acc;
}

int main(void) {
  unsigned s1 = 0, s2 = 0, s3 = 0;
  for (unsigned k = 0; k < 39u; k++) {
    s1 = s1 * 31u + word_pun(k);
    s2 = s2 * 17u + byte_pun(k);
    s3 = s3 * 13u + cmp_pun(k);
  }
  if (s1 != 1124575625u || s2 != 248481280u || s3 != 3238734034u) {
    printf("FAIL i686_narrow_store_alu_slot_source s1=%u s2=%u s3=%u\n"
           "     want s1=1124575625 s2=248481280 s3=3238734034\n",
           s1, s2, s3);
    return 1;
  }
  printf("PASS i686_narrow_store_alu_slot_source\n");
  return 0;
}
