/* Explicit member/record alignment plus complete, empty and flexible arrays. */
struct __attribute__((aligned(0 ?: 64))) audit_record {
    unsigned tag;
    _Alignas(0 ?: 16) unsigned long long value;
    unsigned char empty[0];
    unsigned char single[1];
    unsigned char matrix[2][3];
    unsigned char flexible[];
};
struct audit_natural {
    unsigned tag;
    unsigned payload;
};
struct audit_record record;
struct audit_natural natural_record;

_Static_assert(_Alignof(struct audit_record) == 64, "record alignment");
_Static_assert(sizeof(struct audit_record) == 64, "record size");
_Static_assert(__builtin_offsetof(struct audit_record, value) == 16, "member alignment");
_Static_assert(__builtin_offsetof(struct audit_record, empty) == 24, "empty offset");
_Static_assert(__builtin_offsetof(struct audit_record, single) == 24, "one offset");
_Static_assert(__builtin_offsetof(struct audit_record, matrix) == 25, "matrix offset");
_Static_assert(__builtin_offsetof(struct audit_record, flexible) == 31, "flex offset");
_Static_assert(sizeof record.empty == 0, "empty bound");
_Static_assert(sizeof record.single == 1, "single bound");
_Static_assert(sizeof record.matrix == 6, "matrix bound");

int main(void) {
    volatile unsigned char *base = (volatile unsigned char *)&record;
    record.value = 0x102030405060708ULL;
    record.single[0] = 13;
    record.matrix[1][2] = 41;
    natural_record.payload = 59;
    return base[16] != 8 || base[24] != 13 || base[30] != 41 || natural_record.payload != 59;
}
