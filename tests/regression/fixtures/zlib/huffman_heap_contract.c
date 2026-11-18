/* Drive the benchmark's actual implementation, not a second copy of its heap. */
#define main benchmark_main
#include "../../../benchmark/programs/zlib_ng_huffman_heap.c"
#undef main

static int check_generated_histograms(void)
{
    if (!init_histograms())
        return 0;
    for (int c = 0; c < CASES; ++c) {
        unsigned active = 0;
        uint32_t total = 0;
        for (int n = 0; n < SYMBOLS; ++n) {
            uint16_t weight = histograms[c][n].freq;
            active += weight != 0;
            total += weight;
        }
        if (active < 2 || total > UINT16_MAX) {
            fprintf(stderr, "invalid Huffman histogram %d: %u leaves, %u total\n",
                    c, active, (unsigned)total);
            return 0;
        }
    }
    return 1;
}

static int check_limit_case(int count)
{
    node_t histogram[SYMBOLS] = {0};
    node_t tree[HEAP_SIZE] = {0};
    int ones = count - 1;
    /* A coprime stride scatters unique leaves, with unused symbols between
     * them. The last leaf has >32767 weight even at the largest count. */
    for (int i = 0; i < ones; ++i)
        histogram[(43 * i) % SYMBOLS].freq = 1;
    histogram[(43 * ones) % SYMBOLS].freq = (uint16_t)(UINT16_MAX - ones);
    memcpy(tree, histogram, sizeof histogram);
    int root = huffman_merge(tree);
    if (root != SYMBOLS + count - 2 || tree[root].freq != UINT16_MAX ||
        !check_tree(tree, root, histogram))
        return 0;

    /* Independent closed-form cost. The heavy leaf can only be merged last;
     * the ones form a complete tree with leaves on at most two levels. */
    int power = 1;
    unsigned level = 0;
    while (2 * power <= ones) {
        power *= 2;
        ++level;
    }
    uint32_t expected = UINT16_MAX + (unsigned)ones * level + (unsigned)(2 * (ones - power));
    uint32_t cost = 0;
    for (int n = 0; n < SYMBOLS; ++n) {
        uint16_t weight = histogram[n].freq;
        if (!weight)
            continue;
        for (int child = n; child != root;) {
            int parent = tree[child].dad;
            if (parent < SYMBOLS || parent <= child || parent > root)
                return 0;
            cost += weight;
            child = parent;
        }
    }
    if (cost != expected || check_tree(tree, HEAP_SIZE, histogram))
        return 0;
    --tree[root].freq;
    if (check_tree(tree, root, histogram))
        return 0;
    ++tree[root].freq;
    tree[0].dad = 0; /* active leaf, invalid parent */
    return !check_tree(tree, root, histogram);
}

static int check_degenerate_histograms(void)
{
    node_t tree[HEAP_SIZE] = {0};
    if (check_tree(tree, SYMBOLS - 2, tree))
        return 0;
    tree[SYMBOLS - 1].freq = 1;
    return !check_tree(tree, SYMBOLS - 1, tree);
}

int main(void)
{
    if (!check_generated_histograms())
        return 1;
    if (!check_degenerate_histograms()) {
        fputs("Huffman tree checker accepted fewer than two leaves\n", stderr);
        return 1;
    }
    for (int count = 2; count <= SYMBOLS; ++count) {
        if (!check_limit_case(count)) {
            fprintf(stderr, "Huffman UINT16_MAX histogram failed: %d leaves\n", count);
            return 1;
        }
    }
    return benchmark_main();
}
