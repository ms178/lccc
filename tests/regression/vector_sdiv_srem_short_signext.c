/* Keep narrow signed-vector lanes canonical when x86 scalarizes div/rem.
   A signed i16 home produced with movswl has the right low 32 bits but a zero
   upper half; feeding it straight to idivq turns -1 / -8 into 1 remainder 7. */
typedef short v8hi __attribute__((vector_size(16)));

__attribute__((noinline)) static v8hi
sdiv_neg8(v8hi x)
{
    return x / (v8hi){-8, -8, -8, -8, -8, -8, -8, -8};
}

__attribute__((noinline)) static v8hi
srem_neg8(v8hi x)
{
    return x % (v8hi){-8, -8, -8, -8, -8, -8, -8, -8};
}

int
main(void)
{
    v8hi x = {-1, -7, -8, -9, 1, 7, 8, 9};
    v8hi q = sdiv_neg8(x);
    v8hi r = srem_neg8(x);
    for (int i = 0; i < 8; ++i) {
        if (q[i] != x[i] / -8 || r[i] != x[i] % -8)
            return 1;
    }
    return 0;
}
