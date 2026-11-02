#include <stdio.h>
extern int original_vgatherdps(void), rewritten_vgatherdps(void);
extern int original_vgatherdpd(void), rewritten_vgatherdpd(void);
extern int original_vgatherqps(void), rewritten_vgatherqps(void);
extern int original_vgatherqpd(void), rewritten_vgatherqpd(void);
extern int original_vpgatherdd(void), rewritten_vpgatherdd(void);
extern int original_vpgatherdq(void), rewritten_vpgatherdq(void);
extern int original_vpgatherqd(void), rewritten_vpgatherqd(void);
extern int original_vpgatherqq(void), rewritten_vpgatherqq(void);
int main(void) {
    int bad = 0;
    {
        int a = original_vgatherdps(), b = rewritten_vgatherdps();
        printf("vgatherdps: original=%d rewritten=%d\n", a, b);
        bad |= a != 1 || b != 0;
    }
    {
        int a = original_vgatherdpd(), b = rewritten_vgatherdpd();
        printf("vgatherdpd: original=%d rewritten=%d\n", a, b);
        bad |= a != 1 || b != 0;
    }
    {
        int a = original_vgatherqps(), b = rewritten_vgatherqps();
        printf("vgatherqps: original=%d rewritten=%d\n", a, b);
        bad |= a != 1 || b != 0;
    }
    {
        int a = original_vgatherqpd(), b = rewritten_vgatherqpd();
        printf("vgatherqpd: original=%d rewritten=%d\n", a, b);
        bad |= a != 1 || b != 0;
    }
    {
        int a = original_vpgatherdd(), b = rewritten_vpgatherdd();
        printf("vpgatherdd: original=%d rewritten=%d\n", a, b);
        bad |= a != 1 || b != 0;
    }
    {
        int a = original_vpgatherdq(), b = rewritten_vpgatherdq();
        printf("vpgatherdq: original=%d rewritten=%d\n", a, b);
        bad |= a != 1 || b != 0;
    }
    {
        int a = original_vpgatherqd(), b = rewritten_vpgatherqd();
        printf("vpgatherqd: original=%d rewritten=%d\n", a, b);
        bad |= a != 1 || b != 0;
    }
    {
        int a = original_vpgatherqq(), b = rewritten_vpgatherqq();
        printf("vpgatherqq: original=%d rewritten=%d\n", a, b);
        bad |= a != 1 || b != 0;
    }
    return bad;
}
