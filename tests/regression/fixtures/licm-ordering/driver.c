#include <pthread.h>
#include <stdio.h>
int ordering_g0, ordering_g1, ordering_g2, ordering_g3;
unsigned ordering_ready, ordering_done;
extern int ordering_before_acquire(int);
static void *publish(void *unused) {
    (void)unused;
    while (!__atomic_load_n(&ordering_ready, __ATOMIC_ACQUIRE)) {}
    ordering_g0 = 11; ordering_g1 = 12; ordering_g2 = 13; ordering_g3 = 14;
    __atomic_store_n(&ordering_done, 1, __ATOMIC_RELEASE);
    return 0;
}
int main(void) {
    for (int round = 0; round < 8; ++round) {
        ordering_ready = ordering_done = 0;
        ordering_g0 = 1; ordering_g1 = 2; ordering_g2 = 3; ordering_g3 = 4;
        pthread_t writer;
        if (pthread_create(&writer, 0, publish, 0)) return 2;
        int actual = ordering_before_acquire(2);
        if (pthread_join(writer, 0)) return 3;
        if (actual != 60) { printf("FAIL round=%d result=%d expected=60\n", round, actual); return 1; }
    }
    puts("ordering handshake: 8 rounds pass");
    return 0;
}
