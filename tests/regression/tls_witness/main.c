#include <stdio.h>
#include <pthread.h>
#include <dlfcn.h>
extern __thread int tv;              /* Initial-Exec from the executable */
int get(void); int *addr_tv(void); int bump(void);
long press(long, long, long, long, long, long, long);
static void *thr(void *p) {
    tv = (int)(long)p;
    int b = bump();
    return (void *)(long)(get() * 1000 + b + (addr_tv() == &tv));
}
int main(void) {
    tv += 1;
    int b = bump();
    printf("%d %d %d\n", get(), addr_tv() == &tv, b);         /* 27 1 613 */
    pthread_t t; void *r;
    pthread_create(&t, 0, thr, (void *)40L);
    pthread_join(t, &r);
    printf("%ld %d\n", (long)r, get());                        /* 59614 27 */
    printf("%ld\n", press(2, 3, 5, 7, 11, 13, 17));            /* 284 (tv == 8 here) */
    void *h = dlopen("./libdl_tw.so", RTLD_NOW);
    if (!h) { printf("dlopen: %s\n", dlerror()); return 1; }
    int (*dget)(void) = (int (*)(void))dlsym(h, "dget");
    int d1 = dget();
    printf("%d %d\n", d1, dget());                             /* 101 102 */
    return 0;
}
