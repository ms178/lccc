#include <stdio.h>

extern int original_vucomisd(void), removed_vucomisd(void);
extern int original_vucomiss(void), removed_vucomiss(void);
extern int original_vcomisd(void), removed_vcomisd(void);
extern int original_vcomiss(void), removed_vcomiss(void);
extern int original_vptest(void), removed_vptest(void);
extern int original_vtestpd(void), removed_vtestpd(void);
extern int original_vtestps(void), removed_vtestps(void);
extern int original_vpcmpistri(void), removed_vpcmpistri(void);
extern int original_vpcmpistrm(void), removed_vpcmpistrm(void);
extern int original_vpcmpestri(void), removed_vpcmpestri(void);
extern int original_vpcmpestrm(void), removed_vpcmpestrm(void);

typedef struct {
    const char *name;
    int (*original)(void);
    int (*removed)(void);
} Witness;

static const Witness witnesses[] = {
    { "vucomisd", original_vucomisd, removed_vucomisd },
    { "vucomiss", original_vucomiss, removed_vucomiss },
    { "vcomisd", original_vcomisd, removed_vcomisd },
    { "vcomiss", original_vcomiss, removed_vcomiss },
    { "vptest", original_vptest, removed_vptest },
    { "vtestpd", original_vtestpd, removed_vtestpd },
    { "vtestps", original_vtestps, removed_vtestps },
    { "vpcmpistri", original_vpcmpistri, removed_vpcmpistri },
    { "vpcmpistrm", original_vpcmpistrm, removed_vpcmpistrm },
    { "vpcmpestri", original_vpcmpestri, removed_vpcmpestri },
    { "vpcmpestrm", original_vpcmpestrm, removed_vpcmpestrm },
};

int main(void) {
    int bad = 0;
    for (size_t i = 0; i < sizeof(witnesses) / sizeof(witnesses[0]); ++i) {
        int original = witnesses[i].original();
        int removed = witnesses[i].removed();
        printf("%s: original=%d removed=%d\n", witnesses[i].name, original, removed);
        bad |= original != 1 || removed != 0;
    }
    return bad;
}
