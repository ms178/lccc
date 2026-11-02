#include <stdio.h>

int g(int x) { return x * 3; }
/* cmp -> CALL -> select: operands must survive the call; replay reads post-call. */
int across_call(int a, int b, int c) { int cond = a > b; g(0); return cond ? c : a; }
/* cmp -> many ALU ops -> select */
int far_select(int a, int b) {
    int cond = a >= b;
    int t = a * 3 + b * 7 - (a ^ 5);
    t ^= t >> 3;
    return cond ? t : a - b;
}
/* two cmps, interleaved selects (the clamp family) */
int two_cmp(int a, int b, int c) {
    int c1 = a < b;
    int c2 = b < c;
    int s2 = c2 ? c : b;
    return c1 ? s2 : a;
}
/* cmp -> select where operand is re-staged through slots (force spills) */
int spill_heavy(int a, int b) {
    int cond = a != b;
    int v0=a*1,v1=a*2,v2=a*3,v3=a*4,v4=a*5,v5=a*6,v6=a*7,v7=a*8,v8=a*9;
    int s = v0+v1+v2+v3+v4+v5+v6+v7+v8;
    return cond ? s + b : a - s;
}
/* CondBranch terminator consumer with intervening op */
int branch_far(int a, int b) { int cond = a <= b; int t = a - b * 2; if (cond) return t; return b; }
int main(void) {
    int fails = 0;
    int vals[] = {-2147483647-1, -65536, -101, -3, 0, 2, 17, 101, 65536, 2147483647};
    for (int i = 0; i < 10; i++)
      for (int j = 0; j < 10; j++)
        for (int k = 0; k < 10; k++) {
            int a=vals[i], b=vals[j], c=vals[k];
            if (across_call(a,b,c) != ((a>b)?c:a)) fails++;
            if (far_select(a,b) != ((a>=b)?(a*3+b*7-(a^5))^((a*3+b*7-(a^5))>>3):a-b)) fails++;
            if (two_cmp(a,b,c) != ((a<b)?((b<c)?c:b):a)) fails++;
            if (branch_far(a,b) != ((a<=b)?a-b*2:b)) fails++;
            /* spill_heavy reference */
            {
                int cond=a!=b, s=0;
                for (int m=1;m<=9;m++) s+=a*m;
                if (spill_heavy(a,b) != (cond?s+b:a-s)) fails++;
            }
        }
    printf("fails=%d\n", fails);
    return fails != 0;
}
