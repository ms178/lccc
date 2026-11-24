/* The ordinary operators on `_Atomic` objects are atomic operations (C11
 * 6.5.2.4p2, 6.5.16.2p3): `x++`, `x += v`, `x = v` and a plain read.
 *
 * lccc lowered them as the accesses of a `volatile` object: `x++` on an
 * `atomic_int` was a load, an add and a store, and an update made by
 * another thread in between was lost. Part 1 checks what each operator
 * yields and leaves behind, on every scalar type; part 2 runs the updates
 * from several threads, where one lost update changes the total. */
#include <pthread.h>
#include <stdatomic.h>
#include <stdio.h>

static atomic_int ai;
static _Atomic long al;
static _Atomic unsigned char auc;
static _Atomic signed char asc;
static _Atomic short as;
static _Atomic unsigned long long aull;
static atomic_bool ab;
static _Atomic double ad;
static _Atomic float af;
static int cells[8];
static _Atomic(int *) ap = cells;
static int *_Atomic ap2 = cells + 4;
struct counted { int plain; atomic_int refs; _Atomic long hits; _Atomic(char *) cursor; };
static struct counted obj;
static atomic_int arr[4];
static volatile int one = 1, three = 3, seven = 7;

/* One operator per full expression, results printed in the order made. */
static long long results[24];
static double fresults[24];
static int count;
#define I(e) ((void)(results[count++] = (long long)(e)))
#define F(e) ((void)(fresults[count++] = (double)(e)))
static void show(const char *label) {
    printf("%s:", label);
    for (int i = 0; i < count; i++)
        printf(" %lld", results[i]);
    printf("\n");
    count = 0;
}
static void fshow(const char *label) {
    printf("%s:", label);
    for (int i = 0; i < count; i++)
        printf(" %g", fresults[i]);
    printf("\n");
    count = 0;
}

static void values(void) {
    ai = 10;
    I(ai++); I(++ai); I(ai--); I(--ai); I(ai += three); I(ai -= seven); I(ai *= three);
    I(ai /= three); I(ai %= seven); I(ai <<= three); I(ai >>= one); I(ai |= 0x30);
    I(ai &= 0x3c); I(ai ^= 0xff); I(ai);
    show("int");

    al = 1L << 30;
    I(al++); I(al += 1L << 29); I(al ^= 5); I(al -= seven); I(al *= three); I(al);
    show("long");

    auc = 250;
    I(auc += 10); I(auc++); I(auc *= 100); I(auc -= 200); I(auc <<= one); I(auc);
    show("uchar");
    asc = 126;
    I(++asc); I(++asc); I(asc--); I(asc += 3); I(asc >>= one); I(asc);
    show("schar");
    as = -32768;
    I(as--); I(as += 2); I(as |= 0x700); I(as /= three); I(as);
    show("short");
    aull = ~0ULL;
    I(aull++ == ~0ULL); I(aull -= 2); I(aull >>= 60); I(aull *= 3); I(aull %= seven); I(aull);
    show("ull");

    /* The other operand's type decides the arithmetic: a floating one is
     * not reduced to the object's type first, a wide one is. */
    ai = 7;
    I(ai += 2.75); I(ai *= 1.5); I(ai -= 0.25f); I(ai);
    auc = 200;
    I(auc += 0x1234567); I(auc -= 0x100000001LL); I(auc);
    show("mixed");

    ab = 0;
    I(ab++); I(ab++); I(ab--); I(ab += 0.5); I(ab = 2); I(ab -= one); I(ab ^= 1); I(ab);
    show("bool");

    ad = 1.5;
    F(ad++); F(++ad); F(ad += 0.25); F(ad *= 4); F(ad /= 3); F(ad--); F(ad);
    fshow("double");
    af = 0.5f;
    F(af += 1); F(af++); F(af *= three); F(af -= 0.125); F(--af); F(af);
    fshow("float");

    I(ap++ - cells); I(++ap - cells); I((ap += three) - cells); I((ap -= one) - cells);
    I(ap-- - cells); I(ap - cells);
    I(ap2-- - cells); I((ap2 += 2) - cells); I(ap2 - cells);
    show("pointer");

    obj.refs = 1;
    obj.hits = 100;
    static char text[] = "atomic";
    obj.cursor = text;
    struct counted *o = &obj;
    I(o->refs++); I(--obj.refs); I(o->hits += 5); I(obj.hits <<= 2); I(*o->cursor++);
    I(*++obj.cursor); I(obj.refs); I(o->hits); I(obj.cursor - text);
    show("member");

    int i = 0;
    arr[1] = 5;
    I(arr[i++]++); I(arr[i]--); I(++arr[i++]); I(arr[i] += 9); I(i); I(arr[0]); I(arr[1]);
    I(arr[2]);
    show("element");

    /* An assignment yields the stored value, converted to the object's type. */
    int big = 300;
    I(ai = big); I(auc = big); I(ab = big); I(ai); I(auc); I(ab);
    atomic_int *pi = &ai;
    I((*pi)++); I(*pi += 5); I(*pi = 9); I(*pi);
    show("assign");
    F(ad = 2); F(af = big); F(ad); F(af);
    fshow("fassign");
}

#define THREADS 4
#define ROUNDS 100000

static atomic_int t_inc;
static _Atomic long t_add;
static _Atomic unsigned t_xor;
static _Atomic unsigned t_or;
static _Atomic unsigned long long t_mul;
static _Atomic long long t_wide;
static _Atomic double t_fp;
static _Atomic float t_fp32;
static _Atomic unsigned char t_byte;
static _Atomic short t_shift;
static struct counted t_obj;
static atomic_int t_arr[2];
static _Atomic(char *) t_ptr;
static char arena[THREADS * ROUNDS + 1];

static void *worker(void *arg) {
    unsigned bit = 1u << (unsigned)(long)arg;
    for (int i = 0; i < ROUNDS; i++) {
        t_inc++;
        t_add += 3;
        t_xor ^= bit;       /* ROUNDS is even: every bit ends where it began */
        t_or |= bit;
        t_mul *= 1;
        t_mul += one;
        t_wide -= 0x100000001LL;
        t_fp += 1.0;
        t_fp32 += 0.25f;    /* exact in a float well past this count */
        t_byte -= 1;
        t_shift <<= 0;
        t_shift++;
        t_obj.refs--;
        ++t_obj.hits;
        t_arr[i & 1] += 2;
        *t_ptr++ = 1;       /* each thread claims a byte of its own */
    }
    return 0;
}

static int threads(void) {
    pthread_t tid[THREADS];
    t_ptr = arena;
    t_xor = 0xa5a5a5a5u;
    for (long i = 0; i < THREADS; i++)
        pthread_create(&tid[i], 0, worker, (void *)i);
    for (int i = 0; i < THREADS; i++)
        pthread_join(tid[i], 0);
    long claimed = 0;
    for (long i = 0; i < THREADS * ROUNDS; i++)
        claimed += arena[i];
    printf("threads: %d %ld %x %x %llu %lld %.0f %.0f %d %d %d %ld %d %ld %ld\n", t_inc, t_add,
           t_xor, t_or, t_mul, t_wide, t_fp, (double)t_fp32, t_byte, t_shift, t_obj.refs,
           t_obj.hits, t_arr[0] + t_arr[1], (long)(t_ptr - arena), claimed);
    /* The totals are the program's own to check: a compiler that loses an
     * update fails here whatever it is compared with. */
    const long total = (long)THREADS * ROUNDS;
    return t_inc != total || t_add != 3 * total || t_xor != 0xa5a5a5a5u ||
           t_or != (1u << THREADS) - 1 || t_mul != (unsigned long long)total ||
           t_wide != -0x100000001LL * total || t_fp != (double)total ||
           t_fp32 != 0.25f * (float)total || t_byte != (unsigned char)-total ||
           t_shift != (short)total || t_obj.refs != -total || t_obj.hits != total ||
           t_arr[0] + t_arr[1] != 2 * total || t_ptr - arena != total || claimed != total;
}

int main(void) {
    values();
    return threads();
}
