#include <stdio.h>
#include <string.h>

struct Triple { long lane[3]; };
struct Holder { void *left; void *right; struct Triple value; };

__attribute__((noinline)) static struct Triple make_triple(void) {
    struct Triple result;
    long *cursor = (long *)((char *)&result + 0);
    for (int i = 0; i < 3; ++i)
        *cursor++ = 41 + i;
    return result;
}

__attribute__((noinline)) static void fill(struct Holder *holder) {
    holder->value = make_triple();
}

int main(void) {
    struct Holder holder;
    memset(&holder, 0, sizeof(holder));
    fill(&holder);
    printf("%ld %ld %ld\n", holder.value.lane[0], holder.value.lane[1], holder.value.lane[2]);
    return !(holder.value.lane[0] == 41 && holder.value.lane[1] == 42 && holder.value.lane[2] == 43);
}
