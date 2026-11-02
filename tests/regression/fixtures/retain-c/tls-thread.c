#include <pthread.h>
__thread int tls_initialized __attribute__((retain)) = 17;
__thread int tls_zero __attribute__((retain));
__thread int tls_custom __attribute__((retain, section(".tls_custom"))) = 23;
static void *worker(void *arg) {
    int *result = (int *)arg;
    if (tls_initialized != 17 || tls_zero != 0 || tls_custom != 23) {
        *result = 1;
        return 0;
    }
    tls_initialized = *result;
    tls_zero = *result + 1;
    tls_custom = *result + 2;
    if (tls_initialized != *result || tls_zero != *result + 1 || tls_custom != *result + 2)
        *result = 2;
    else
        *result = 0;
    return 0;
}
int main(void) {
    pthread_t a, b;
    int x = 61, y = 73;
    if (pthread_create(&a, 0, worker, &x) || pthread_create(&b, 0, worker, &y)) return 3;
    if (pthread_join(a, 0) || pthread_join(b, 0)) return 4;
    return x || y || tls_initialized != 17 || tls_zero != 0 || tls_custom != 23;
}
