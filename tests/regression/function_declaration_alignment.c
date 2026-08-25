/* A function alignment attribute on a prototype contributes both to GNU
 * __alignof__(function) and to final code placement of the later definition. */
#include <stdint.h>
#include <stdlib.h>

static void aligned_function(void) __attribute__((aligned(256)));
static void aligned_function(void) {}

int main(void)
{
    if (__alignof__(aligned_function) != 256)
        abort();
    if (((uintptr_t)&aligned_function & 255U) != 0)
        abort();
    return 0;
}
