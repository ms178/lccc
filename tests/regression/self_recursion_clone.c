/* One-level self-recursion clone: shapes the codegen gate pins.
 *
 * Every function here is directly self-recursive. The gate
 * (check_self_recursion_clone.sh) reads the emitted assembly and requires
 * that
 *
 *   `walk_tree` / `count_tree` / `free_tree` — small, *non-tail* self calls —
 *   are cloned exactly one level (the call/return of one level disappears),
 *   while `tail_sum`, whose self call is in tail position, keeps its single
 *   call because `tail_call_elim` turns it into a loop.
 *
 * The main function prints a deterministic checksum so the same gate can also
 * prove that cloning is semantics-preserving: the clone-on and clone-off
 * binaries must print identical output.
 */
#include <stdio.h>
#include <stdlib.h>

typedef struct Node {
    struct Node *left, *right;
    int value;
} Node;

/* Non-tail double recursion (the binary_trees make/check/destroy shape). */
static Node *walk_tree(int depth) {
    Node *n = (Node *)malloc(sizeof(Node));
    n->value = depth;
    if (depth > 0) {
        n->left = walk_tree(depth - 1);
        n->right = walk_tree(depth - 1);
    } else {
        n->left = n->right = NULL;
    }
    return n;
}

/* Non-tail recursion with work after the call (a phi-carrying accumulator). */
static int count_tree(Node *n) {
    if (n->left == NULL) return 1;
    return 1 + count_tree(n->left) + count_tree(n->right);
}

/* Non-tail recursion whose only call is the last instruction of a block. */
static void free_tree(Node *n) {
    if (n->left) {
        free_tree(n->left);
        free_tree(n->right);
    }
    free(n);
}


/* Large non-tail recursion: the one-level clone is bounded by
   MAX_SELF_CLONE_INSTRUCTIONS, so this body must stay a single call. */
static unsigned big_walk(int depth, unsigned seed) {
    /* Unsigned arithmetic: the wrap is defined, so every compiler must agree
       on the checksum. (Signed overflow here was UB and the corpus
       differential rejected it: lccc and GCC printed different values.) */
    unsigned v = seed;
    v = v * 1103515245 + 12345;
    v ^= v >> 7;
    v = v * 1103515245 + 12346;
    v ^= v >> 8;
    v = v * 1103515245 + 12347;
    v ^= v >> 9;
    v = v * 1103515245 + 12348;
    v ^= v >> 10;
    v = v * 1103515245 + 12349;
    v ^= v >> 11;
    v = v * 1103515245 + 12350;
    v ^= v >> 12;
    v = v * 1103515245 + 12351;
    v ^= v >> 13;
    v = v * 1103515245 + 12352;
    v ^= v >> 14;
    v = v * 1103515245 + 12353;
    v ^= v >> 15;
    v = v * 1103515245 + 12354;
    v ^= v >> 16;
    v = v * 1103515245 + 12355;
    v ^= v >> 17;
    v = v * 1103515245 + 12356;
    v ^= v >> 18;
    v = v * 1103515245 + 12357;
    v ^= v >> 19;
    v = v * 1103515245 + 12358;
    v ^= v >> 7;
    v = v * 1103515245 + 12359;
    v ^= v >> 8;
    v = v * 1103515245 + 12360;
    v ^= v >> 9;
    v = v * 1103515245 + 12361;
    v ^= v >> 10;
    v = v * 1103515245 + 12362;
    v ^= v >> 11;
    v = v * 1103515245 + 12363;
    v ^= v >> 12;
    v = v * 1103515245 + 12364;
    v ^= v >> 13;
    v = v * 1103515245 + 12365;
    v ^= v >> 14;
    v = v * 1103515245 + 12366;
    v ^= v >> 15;
    v = v * 1103515245 + 12367;
    v ^= v >> 16;
    v = v * 1103515245 + 12368;
    v ^= v >> 17;
    v = v * 1103515245 + 12369;
    v ^= v >> 18;
    v = v * 1103515245 + 12370;
    v ^= v >> 19;
    v = v * 1103515245 + 12371;
    v ^= v >> 7;
    v = v * 1103515245 + 12372;
    v ^= v >> 8;
    v = v * 1103515245 + 12373;
    v ^= v >> 9;
    v = v * 1103515245 + 12374;
    v ^= v >> 10;
    v = v * 1103515245 + 12375;
    v ^= v >> 11;
    v = v * 1103515245 + 12376;
    v ^= v >> 12;
    v = v * 1103515245 + 12377;
    v ^= v >> 13;
    v = v * 1103515245 + 12378;
    v ^= v >> 14;
    v = v * 1103515245 + 12379;
    v ^= v >> 15;
    v = v * 1103515245 + 12380;
    v ^= v >> 16;
    v = v * 1103515245 + 12381;
    v ^= v >> 17;
    v = v * 1103515245 + 12382;
    v ^= v >> 18;
    v = v * 1103515245 + 12383;
    v ^= v >> 19;
    v = v * 1103515245 + 12384;
    v ^= v >> 7;
    if (depth <= 0) return v;
    return v + big_walk(depth - 1, seed + 1u);
}

/* Tail recursion: `tail_call_elim` converts this to a loop, so the cloner
   must leave it entirely alone. */
__attribute__((noinline)) static long tail_sum(int n, long acc) {
    if (n <= 0) return acc;
    return tail_sum(n - 1, acc + n);
}

int main(void) {
    /* A `volatile` seed keeps both outline-only functions away from the
       compile-time constant fold: a fully folded call removes the function
       from the assembly and would make the size/tail gates vacuous. */
    volatile int seed = 24;
    unsigned long long sum = 0;
    for (int depth = 1; depth <= 14; depth++) {
        Node *t = walk_tree(depth);
        sum = sum * 31u + (unsigned long long)count_tree(t);
        free_tree(t);
    }
    sum += (unsigned long long)tail_sum(seed * 170, 0);
    /* Two call sites keep `big_walk` outlined: a single-site static body is
       inlined by the called-once rule, which would hide the size gate. */
    sum += (unsigned long long)big_walk(seed, 7u);
    sum += (unsigned long long)big_walk(seed - 21, 11u);
    printf("%llu\n", sum);
    return 0;
}
