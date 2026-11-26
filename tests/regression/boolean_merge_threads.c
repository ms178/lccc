/* A condition in a function of its own, branched on where it is inlined.
 *
 *     static int smaller(...) { return a < b || (a == b && c <= d); }
 *     ...
 *     if (smaller(...)) j++;
 *
 * The value of `||` is a merge of the constant 1 and what the right side
 * computes, and the caller branches on it: the way in that brings the 1
 * knows which way the branch goes.  If-conversion leaves such a branch
 * (`if_convert::branch_is_decided_on_entry`) and the decided-branch threader
 * sends that way in straight to its arm, behind the optimizer's rounds
 * (`jump_thread::thread_decided`), so that no 1 is written to be tested and
 * no `j + 1` is computed to be discarded.  Before, if-conversion made the
 * branch a select: zlib's `pqdownheap` (tests/benchmark/programs/
 * zlib_ng_huffman_heap.c, the first function below) ran `movl $1, %r12d;
 * leal 1(%rdx), %r9d; testq %r12, %r12; cmovne` on the path that had just
 * decided to step.
 *
 * A condition built from conditions is the same merge a level deeper, and
 * the threads are taken a level a round.  What is left of a character class
 * is then a chain of tests whose hits all go to one place, and the set fold
 * that follows the threads makes it one range test and one bit test: the
 * `nest_*` functions.
 *
 * Read by check_boolean_merge_threads.py: the output against GCC's at every
 * level, and at -O2 the assembly of the functions it names (with the early
 * position of the threader off the pins have to break).  The functions
 * named `keep_*` are the ones a wrong answer would come from: a constant
 * that is not 0 or 1 behind a narrowing cast, a value that is read again
 * behind the branch, a merge that is a loop's own test.
 */
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>

struct ct {
    uint16_t freq;
    uint16_t code;
};

#define NOINLINE __attribute__((noinline))

static inline int smaller(const struct ct *tree, int n, int m, const uint8_t *depth) {
    return tree[n].freq < tree[m].freq ||
           (tree[n].freq == tree[m].freq && depth[n] <= depth[m]);
}

/* zlib's pqdownheap: two merges a level, one steps `j`, one leaves. */
NOINLINE void merge_sift_down(const struct ct *tree, int *heap, const uint8_t *depth,
                              int heap_len, int k) {
    int v = heap[k];
    int j = k << 1;
    while (j <= heap_len) {
        if (j < heap_len && smaller(tree, heap[j + 1], heap[j], depth)) {
            j++;
        }
        if (smaller(tree, v, heap[j], depth)) {
            break;
        }
        heap[k] = heap[j];
        k = j;
        j <<= 1;
    }
    heap[k] = v;
}

static inline int before(int a, int b, int c, int d) {
    return a < b || (a == b && c <= d);
}

/* The merge guards a step of a counter. */
NOINLINE int merge_count_before(const int *a, const int *b, int n) {
    int count = 0;
    for (int i = 0; i + 1 < n; i++) {
        if (before(a[i], a[i + 1], b[i], b[i + 1])) {
            count++;
        }
    }
    return count;
}

static inline int inside(int x, int lo, int hi) {
    return x >= lo && x < hi;
}

/* `&&`: the constant is the 0 the first test brings. */
NOINLINE long merge_sum_inside(const int *a, int n, int lo, int hi) {
    long total = 0;
    for (int i = 0; i < n; i++) {
        if (inside(a[i], lo, hi)) {
            total += a[i];
        }
    }
    return total;
}

/* The negation of a merge, and a call on each side. */
NOINLINE void sink(int *slot, int x) {
    *slot = (int)((unsigned)*slot * 31u + (unsigned)x);
}

NOINLINE void merge_route(const int *a, const int *b, int n, int *low, int *high) {
    for (int i = 0; i < n; i++) {
        if (!before(a[i], b[i], i, n - i)) {
            sink(high, a[i]);
        } else {
            sink(low, b[i]);
        }
    }
}

/* The merge leaves a loop. */
NOINLINE int merge_find_first(const int *a, int n, int x, int y) {
    int i;
    for (i = 0; i < n; i++) {
        if (before(a[i], x, i, y)) {
            break;
        }
    }
    return i;
}

/* Three levels: a merge of a merge of a merge. */
static inline int ordered(int a, int b, int c, int d, int e, int f) {
    return a < b || (a == b && (c < d || (c == d && e <= f)));
}

NOINLINE int merge_count_ordered(const int *a, const int *b, const int *c, int n) {
    int count = 0;
    for (int i = 0; i + 1 < n; i++) {
        if (ordered(a[i], a[i + 1], b[i], b[i + 1], c[i], c[i + 1])) {
            count += 3;
        } else {
            count -= 1;
        }
    }
    return count;
}

/* -- conditions built from conditions, over one character ---------------- */

static inline int blank(int c) {
    return c == ' ' || c == '\t';
}

static inline int line_break(int c) {
    return c == '\n' || c == '\r' || c == '\f' || c == '\v';
}

static inline int space(int c) {
    return blank(c) || line_break(c);
}

static inline int separator(int c) {
    return space(c) || c == ',' || c == ';';
}

/* Three levels, the innermost handed on as a value twice. */
NOINLINE size_t nest_skip_separators(const char *s, size_t n) {
    size_t i = 0;
    while (i < n && separator(s[i])) {
        i++;
    }
    return i;
}

NOINLINE int nest_token_end(int c) {
    if (separator(c) || c == 0) {
        return 1;
    }
    return 0;
}

static inline int sparse(int c) {
    return c == ' ' || c == '\t' || c == '\r' || c == ',' || c == ';' || c == '#';
}

/* One level: the hits of the helper are at the loop's step only once its
 * merge is threaded. */
NOINLINE size_t nest_skip_sparse(const char *s, size_t n) {
    size_t i = 0;
    while (i < n && sparse(s[i])) {
        i++;
    }
    return i;
}

static inline int inner(int c) {
    return c == ' ' || c == '\t' || c == '\r';
}

static inline int outer(int c) {
    return inner(c) || c == ',' || c == ';' || c == '#';
}

/* Two levels, the inner one first: its merge branches into the outer one. */
NOINLINE size_t nest_skip_nested(const char *s, size_t n) {
    size_t i = 0;
    while (i < n && outer(s[i])) {
        i++;
    }
    return i;
}

/* What the helper returns stands between the merge and its branch: nothing
 * for `int`, `!= 0` for `_Bool`, a narrowing cast for `unsigned char`.  The
 * constant is folded through it and the comparison is read through it. */
static inline _Bool sparse_flag(int c) {
    return c == ' ' || c == '\t' || c == '\r' || c == ',' || c == ';' || c == '#';
}

NOINLINE size_t nest_skip_sparse_bool(const char *s, size_t n) {
    size_t i = 0;
    while (i < n && sparse_flag(s[i])) {
        i++;
    }
    return i;
}

static inline unsigned char inner_byte(int c) {
    return c == ' ' || c == '\t' || c == '\r';
}

static inline unsigned char outer_byte(int c) {
    return inner_byte(c) || c == ',' || c == ';' || c == '#';
}

NOINLINE size_t nest_skip_nested_byte(const char *s, size_t n) {
    size_t i = 0;
    while (i < n && outer_byte(s[i])) {
        i++;
    }
    return i;
}

static inline int operator_char(int c) {
    return c == '+' || c == '-' || c == '*' || c == '/';
}

static inline int punctuation(int c) {
    return operator_char(c) || c == '(' || c == ')' || c == '=';
}

/* The merge guards a step of a counter. */
NOINLINE size_t nest_count_punctuation(const char *s, size_t n) {
    size_t count = 0;
    for (size_t i = 0; i < n; i++) {
        if (punctuation(s[i])) {
            count++;
        }
    }
    return count;
}

static inline int excluded(unsigned x) {
    return x == 3 || x == 7 || x == 11;
}

static inline int wanted(unsigned x, unsigned lo, unsigned hi) {
    return x >= lo && x < hi && !excluded(x);
}

/* `&&` of a negated `||`: the constants are 0s. */
NOINLINE unsigned nest_sum_wanted(const unsigned *a, size_t n, unsigned lo, unsigned hi) {
    unsigned total = 0;
    for (size_t i = 0; i < n; i++) {
        if (wanted(a[i], lo, hi)) {
            total += a[i];
        }
    }
    return total;
}

/* -- what a wrong answer would come from -------------------------------- */

static inline int wide(int a, int b) {
    return a < b ? 256 : a == b;
}

/* 256 is true, and as an `unsigned char` it is 0: the path that brings it
 * must go where 0 goes. */
NOINLINE int keep_narrowed(int a, int b) {
    unsigned char t = (unsigned char)wide(a, b);
    if (t) {
        return 10;
    }
    return 20;
}

/* The same constant read at full width is true. */
NOINLINE int keep_wide(int a, int b) {
    long t = wide(a, b);
    if (t) {
        return 10;
    }
    return 20;
}

/* -1 through a widening, and through `_Bool`. */
static inline int minus(int a, int b) {
    return a < b ? -1 : a - b > 7;
}

NOINLINE int keep_signs(int a, int b) {
    long long wider = minus(a, b);
    _Bool flag = minus(b, a);
    unsigned short low = (unsigned short)((unsigned)minus(a, b) << 16);
    return (wider ? 1 : 0) + (flag ? 2 : 0) + (low ? 4 : 0);
}

/* The value is read again behind the branch, by both sides. */
NOINLINE int keep_value(int a, int b, int c, int *out) {
    int ok = before(a, b, c, *out);
    if (ok) {
        *out += a;
    } else {
        *out -= b;
    }
    return ok * 5 + *out;
}

/* The merge is a loop's own test: the loop keeps its header. */
static inline int at_blank(const unsigned char *p, const unsigned char *end) {
    return p < end && (*p == ' ' || *p == '\t');
}

NOINLINE int keep_loop_test(const unsigned char *p, const unsigned char *end) {
    const unsigned char *start = p;
    while (at_blank(p, end)) {
        p++;
    }
    return (int)(p - start);
}

/* `continue` from a merge: the path it decides goes to the loop's header. */
NOINLINE long keep_continue(const int *a, const int *b, int n) {
    long total = 0;
    for (int i = 0; i < n; i++) {
        if (before(a[i], b[i], i, n)) {
            continue;
        }
        total += a[i] - b[i];
    }
    return total;
}

/* A merge whose constants are a loop's entry: `for (ok = 1; ok; ...)`. */
NOINLINE int keep_flag_loop(const int *a, int n) {
    int i = 0;
    int ok;
    for (ok = 1; ok; ok = i < n && a[i] >= 0) {
        i++;
    }
    return i;
}

/* -- driver --------------------------------------------------------------- */

static uint64_t state = 0x9e3779b97f4a7c15ull;

static uint32_t next(void) {
    state ^= state << 13;
    state ^= state >> 7;
    state ^= state << 17;
    return (uint32_t)(state >> 11);
}

static uint64_t sum = 0xcbf29ce484222325ull;

static void mix(uint64_t x) {
    sum = (sum ^ x) * 0x100000001b3ull;
}

#define N 96

int main(void) {
    static struct ct tree[N];
    static uint8_t depth[N];
    static int heap[N + 2], a[N], b[N], c[N];
    static unsigned char text[N];
    static char line[N];
    static unsigned numbers[N];
    /* Every character the classes name, their neighbours, and bytes that
     * are negative as a `char`. */
    static const char alphabet[] = " \t\n\r\f\v,;#+-*/()=\b\016!\"$'.0:<>ax\177\200\240\377";

    for (int round = 0; round < 200; round++) {
        /* Few distinct values: ties, which the second and third tests see. */
        int spread = 2 + (int)(next() % 5);
        for (int i = 0; i < N; i++) {
            tree[i].freq = (uint16_t)(next() % spread);
            depth[i] = (uint8_t)(next() % 3);
            a[i] = (int)(next() % spread) - 1;
            b[i] = (int)(next() % spread) - 1;
            c[i] = (int)(next() % 3);
            text[i] = (unsigned char)" \t x"[next() % 4];
            /* Runs of class members, so that the scans get somewhere. */
            line[i] = alphabet[next() % ((round & 1) ? 9 : sizeof alphabet - 1)];
            numbers[i] = next() % 16;
        }
        int heap_len = 1 + (int)(next() % (N - 2));
        for (int i = 1; i <= heap_len; i++) {
            heap[i] = (int)(next() % N);
        }
        for (int k = heap_len / 2; k >= 1; k--) {
            merge_sift_down(tree, heap, depth, heap_len, k);
        }
        for (int i = 1; i <= heap_len; i++) {
            mix((uint64_t)heap[i]);
        }

        int n = (int)(next() % (N + 1));
        mix((uint64_t)merge_count_before(a, b, n));
        /* One call of `next` a statement: the order of two in one argument
         * list is the compiler's to choose. */
        int lo = (int)(next() % 3) - 1;
        int hi = (int)(next() % 4);
        mix((uint64_t)merge_sum_inside(a, n, lo, hi));
        int low = 1, high = 2;
        merge_route(a, b, n, &low, &high);
        mix((uint64_t)(uint32_t)low);
        mix((uint64_t)(uint32_t)high);
        int x = (int)(next() % 3) - 1;
        int y = (int)(next() % N);
        mix((uint64_t)merge_find_first(a, n, x, y));
        mix((uint64_t)merge_count_ordered(a, b, c, n));

        for (size_t from = 0; from < N; from += 7) {
            mix(nest_skip_separators(line + from, N - from));
            mix(nest_skip_sparse(line + from, N - from));
            mix(nest_skip_nested(line + from, N - from));
            mix(nest_skip_sparse_bool(line + from, N - from));
            mix(nest_skip_nested_byte(line + from, N - from));
        }
        mix(nest_count_punctuation(line, (size_t)n));
        unsigned first = next() % 6;
        unsigned last = first + next() % 12;
        mix(nest_sum_wanted(numbers, (size_t)n, first, last));
        for (int ch = -130; ch < 260; ch++) {
            mix((uint64_t)nest_token_end(ch));
        }

        for (int i = 0; i + 1 < N; i += 2) {
            mix((uint64_t)keep_narrowed(a[i], b[i]));
            mix((uint64_t)keep_wide(a[i], b[i]));
            mix((uint64_t)keep_signs(a[i] * 5, b[i] * 5));
            int out = c[i] - 1;
            mix((uint64_t)keep_value(a[i], b[i], c[i + 1], &out));
            mix((uint64_t)(uint32_t)out);
        }
        int from = (int)(next() % N);
        mix((uint64_t)keep_loop_test(text + from, text + N));
        mix((uint64_t)keep_continue(a, b, n));
        a[next() % N] = -5;
        mix((uint64_t)keep_flag_loop(a, n));
    }
    printf("boolean merges %016llx\n", (unsigned long long)sum);
    return 0;
}
