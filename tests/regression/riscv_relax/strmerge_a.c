/*
 * GNU ld merges allocated SHF_MERGE inputs (bfd/merge.c): duplicate strings
 * across objects, strings that are a suffix of another (tail merging), and
 * fixed-size constants (.rodata.cst8/.cst16, .srodata.cst8). Linked with
 * strmerge_b.c and strmerge_c.s; the image must equal GNU ld's byte for byte.
 */
typedef double v2df __attribute__((vector_size(16)));

extern const char *b_pick(int i);
extern double b_scale(double x);
extern const char gstr[];

const char *volatile a_sink;
double volatile a_dsink;
long volatile a_lsink;

__attribute__((noinline)) static void put(const char *s) { a_sink = s; }

__attribute__((noinline)) v2df a_vec(v2df x)
{
    return x * (v2df){1.25, -3.5};
}

void _start(void)
{
    put("shared format %d\n");
    put("format %d\n");          /* tail of the first */
    put("only in a");
    put("ending");
    put(gstr);
    put(b_pick(0));
    put(b_pick(1));
    a_dsink = b_scale(2.0) * 2.718281828459045 + 0.1;
    a_lsink = 0x123456789abcdefL;
    a_vec((v2df){a_dsink, 1.0});
    for (;;)
        ;
}
