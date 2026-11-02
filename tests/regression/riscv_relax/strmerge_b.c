/* Second input of the strmerge family (see strmerge_a.c). */
static const char *const table[] = {"shared format %d\n", "nd b"};

const char *b_pick(int i)
{
    return table[i & 1];
}

double b_scale(double x)
{
    return x * 2.718281828459045 + 0.1;
}

const char *b_tail(void)
{
    return "only in b, ending";
}
