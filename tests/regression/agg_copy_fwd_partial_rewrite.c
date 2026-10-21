/* A whole-object copy stays when a partial write redefines only part of the
 * object before the object is read again.
 *
 *   struct Big tmp = s;   // whole-object copy: the only definition of .tail
 *   tmp.h = h;            // redefines .h only, 16 bytes of a 48-byte object
 *   use(tmp.tail);        // still needs what the copy put there
 *
 * sret_copy_elision's forwarding phase listed every Memcpy whose destination
 * derived to an object as a re-initialization of that object, without checking
 * that the copy covered it, and gave each copy ownership of the reads up to
 * the next such copy. Here the second copy fills the first 16 bytes, so the
 * first copy owned no reads at all, was deleted as dead, and `.tail` came back
 * uninitialized.
 *
 * The shape reached production through a struct-returning helper whose return
 * value is a vector of long doubles (`r = d; r[0] = ...; return r;` lowers to
 * a 32-byte whole-object copy followed by a 10-byte copy). That path is
 * covered by the ext-vector ABI grid; this file pins the same defect through
 * portable C, where a member struct assignment is the partial write.
 *
 * Reads `.tail`, i.e. bytes the partial copy does not touch, so a miscompile
 * returns an unrelated value rather than a plausible one. */
#include <stdio.h>
#include <string.h>

typedef struct H {
    long long a, b;
} H;

typedef struct Big {
    H h;
    long long tail[4];
} Big;

static long long __attribute__((noinline)) f(Big s, H h) {
    Big tmp = s;
    tmp.h = h;
    return tmp.tail[0] * 1000000 + tmp.tail[1] * 1000 + tmp.tail[2] + tmp.tail[3];
}

int main(void) {
    Big s;
    memset(&s, 0, sizeof s);
    s.h.a = 1;
    s.h.b = 2;
    s.tail[0] = 111;
    s.tail[1] = 222;
    s.tail[2] = 333;
    s.tail[3] = 444;
    H h = { 7, 8 };
    long long r = f(s, h);
    printf("r=%lld\n", r);
    return r == 111222333 + 444 ? 0 : 1;
}
