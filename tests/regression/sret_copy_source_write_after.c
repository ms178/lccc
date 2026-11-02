/* A forwarded read must observe the bytes the copy took, not whatever the
 * source holds by the time the read runs.
 *
 *   struct S d = src;    // whole-object copy: the pass wants to re-root the
 *                        // reads of `d` at `src` and delete the copy
 *   src.a = 5;           // partial write to the SOURCE, between copy and read
 *   use(d.a);            // must still be the copied value
 *
 * sret_copy_elision's forwarding phase proved the source stable against
 * *whole-object* copies into it only: its `writes` map recorded `Reinit`
 * coordinates and the source rule asked whether each of those dominated the
 * copy. A field store, a partial copy or an aliased store left no coordinate
 * at all, so the copy looked safe, the load was re-rooted at the source, and
 * it came back with the newer value. Before the fix, `field_store` returned 5
 * and `aliased_store` returned 11 where GCC returns 1 and 7.
 *
 * The stale value is not always visible in the output. When the written field
 * is a count, a length or a limit, the read only changes how much work is
 * done, which is how the same defect also presents as a pure performance
 * regression rather than a wrong answer. `written_before` is the positive
 * control: a source written *before* the copy must still be forwarded, so the
 * file keeps failing if the rule ever turns into a blanket refusal. */
#include <stdio.h>
#include <string.h>

typedef struct {
    long a, b, c;
} S;

static int failures;

static void check(const char *name, long got, long want) {
    int failed = got != want;
    failures += failed;
    printf("%s %s got=%ld want=%ld\n", name, failed ? "FAIL" : "ok", got, want);
}

/* Partial field store to the source after the copy. */
static long field_store(long seed) {
    S src;
    src.a = seed;
    src.b = 2;
    src.c = 3;
    S d = src;
    src.a = 5;
    return d.a;
}

/* Full-width store through an aliasing pointer after the copy: no derived
 * path names the written bytes, so only the recorded write position can. */
static long aliased_store(long seed) {
    S src;
    S d;
    src.a = seed;
    src.b = 8;
    src.c = 9;
    d = src;
    *(long *)&src = 11;
    return d.a;
}

/* Positive control: the source is written before the copy, so the copy takes
 * the new value and the forwarded read must see exactly that. */
static long written_before(long seed) {
    S src;
    src.a = 1;
    src.b = 2;
    src.c = 3;
    src.a = seed;
    S d = src;
    return d.a;
}

int main(void) {
    check("field_store", field_store(1), 1);
    check("aliased_store", aliased_store(7), 7);
    check("written_before", written_before(34), 34);
    return failures != 0;
}
