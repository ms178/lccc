/* A snapshot must not be forwarded past an aliasing write or call. */
typedef unsigned long long word;
typedef struct { word a, b, c, d, e, f, g, h; } record;

__attribute__((noinline)) static void mutate(record *p)
{
    p->a = 100;
    p->b = 200;
}

__attribute__((noinline)) static void snapshot_call(record *out, record *in)
{
    record snapshot = *in;
    mutate(in);
    *out = snapshot;
}

__attribute__((noinline)) static void snapshot_alias(record *out, record *in, word *alias)
{
    record snapshot = *in;
    *alias = 300;
    *out = snapshot;
}

__attribute__((noinline)) static void snapshot_local(record *out)
{
    record in = {1, 2, 3, 4, 5, 6, 7, 8};
    record snapshot = in;
    mutate(&in);
    *out = snapshot;
}

/* The large-copy forwarding path has its own memory-effect checks. */
typedef struct { unsigned int value; unsigned char padding[124]; } large_record;
__attribute__((noinline)) static unsigned int snapshot_atomic(void)
{
    large_record source = { .value = 1 }, snapshot;
    __builtin_memcpy(&snapshot, &source, sizeof source);
    __atomic_store_n(&source.value, 2, __ATOMIC_RELAXED);
    return snapshot.value;
}

int main(void)
{
    record x = {1, 2, 3, 4, 5, 6, 7, 8}, out = {0};
    snapshot_call(&out, &x);
    if (out.a != 1 || out.b != 2 || x.a != 100 || x.b != 200) return 1;
    x.a = 1; x.b = 2;
    snapshot_alias(&out, &x, &x.a);
    if (out.a != 1 || out.b != 2 || x.a != 300) return 2;
    snapshot_local(&out);
    if (out.a != 1 || out.b != 2 || out.h != 8) return 3;
    if (snapshot_atomic() != 1) return 4;
    return 0;
}
