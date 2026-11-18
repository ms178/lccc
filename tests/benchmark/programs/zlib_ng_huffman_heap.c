/* SPDX-License-Identifier: Zlib */
/* Adapted from zlib-ng 2.3.3 trees.c: smaller, pqdownheap, and build_tree's
 * heap/merge stage. Copyright (C) 1995-2024 Jean-loup Gailly.
 * License: third_party_licenses/Zlib.txt.
 * This is an altered, standalone benchmark, not a Deflate encoder. It retains
 * the one-based heap, frequency/depth tie rule and parent stores. It omits the
 * workspace's sorted tail, length limiting, and bit-code generation; fixtures
 * already have at least two active symbols and a total frequency <= UINT16_MAX.
 */
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#ifndef PASSES
#define PASSES 768
#endif

enum { SYMBOLS = 286, HEAP_SIZE = 2 * SYMBOLS + 1, CASES = 16 };

typedef struct {
    uint16_t freq;
    uint16_t dad;
} node_t;

static node_t histograms[CASES][SYMBOLS];

static inline int smaller(const node_t *tree, int n, int m, const unsigned char *depth)
{
    return tree[n].freq < tree[m].freq
        || (tree[n].freq == tree[m].freq && depth[n] <= depth[m]);
}

static inline void pqdownheap(const unsigned char *depth, int *heap, int heap_len,
                              const node_t *tree, int k)
{
    int j = k << 1;
    const int v = heap[k];
    while (j <= heap_len) {
        if (j < heap_len && smaller(tree, heap[j + 1], heap[j], depth))
            j++;
        if (smaller(tree, v, heap[j], depth))
            break;
        heap[k] = heap[j];
        k = j;
        j <<= 1;
    }
    heap[k] = v;
}

__attribute__((noinline))
static int huffman_merge(node_t *tree)
{
    unsigned char depth[HEAP_SIZE];
    int heap[HEAP_SIZE];
    int len = 0;
    for (int n = 0; n < SYMBOLS; ++n) {
        if (tree[n].freq != 0) {
            heap[++len] = n;
            depth[n] = 0;
        }
    }
    for (int n = len / 2; n >= 1; --n)
        pqdownheap(depth, heap, len, tree, n);

    for (int node = SYMBOLS; len >= 2; ++node) {
        int n = heap[1];
        heap[1] = heap[len--];
        pqdownheap(depth, heap, len, tree, 1);
        int m = heap[1];
        tree[node].freq = (uint16_t)(tree[n].freq + tree[m].freq);
        depth[node] = (unsigned char)((depth[n] >= depth[m] ? depth[n] : depth[m]) + 1);
        tree[n].dad = tree[m].dad = (uint16_t)node;
        heap[1] = node;
        pqdownheap(depth, heap, len, tree, 1);
    }
    return heap[1];
}

/* Compute the optimal weighted length by choosing the two lowest weights
 * from an unordered list. This deliberately shares neither the heap nor its
 * comparator; equal-weight choices need not produce the same tree. */
static uint32_t reference_cost(uint32_t *weights, int len)
{
    uint32_t cost = 0;
    while (len >= 2) {
        int first = 0;
        for (int n = 1; n < len; ++n)
            if (weights[n] < weights[first])
                first = n;
        uint32_t least = weights[first];
        weights[first] = weights[--len];
        int second = 0;
        for (int n = 1; n < len; ++n)
            if (weights[n] < weights[second])
                second = n;
        weights[second] += least;
        cost += weights[second];
    }
    return cost;
}

static int check_tree(const node_t *tree, int root, const node_t *histogram)
{
    uint32_t weights[SYMBOLS];
    uint32_t child_sum[HEAP_SIZE] = {0};
    unsigned char have_children[HEAP_SIZE] = {0};
    int active = 0;
    uint32_t total = 0;
    uint32_t cost = 0;
    for (int n = 0; n < SYMBOLS; ++n) {
        if (tree[n].freq != histogram[n].freq)
            return 0;
        if (!histogram[n].freq)
            continue;
        total += histogram[n].freq;
        weights[active++] = histogram[n].freq;
    }
    if (root != SYMBOLS + active - 2 || (uint32_t)tree[root].freq != total)
        return 0;
    for (int n = 0; n < root; ++n) {
        if (n < SYMBOLS && !histogram[n].freq)
            continue;
        int dad = tree[n].dad;
        if (dad < SYMBOLS || dad <= n || dad > root || ++have_children[dad] > 2)
            return 0;
        child_sum[dad] += tree[n].freq;
    }
    for (int n = SYMBOLS; n <= root; ++n) {
        if (have_children[n] != 2 || child_sum[n] != (uint32_t)tree[n].freq)
            return 0;
        cost += child_sum[n];
    }
    return cost == reference_cost(weights, active);
}

/* The cost oracle cannot detect a tie-break change. Check the exact heap
 * output when children tie, when their depths differ, and when v ties too. */
static int check_ties(void)
{
    node_t tree[] = {{3, 0}, {1, 0}, {1, 0}};
    unsigned char depth[] = {0, 0, 0};
    int heap[] = {-1, 0, 1, 2};
    pqdownheap(depth, heap, 3, tree, 1);
    if (heap[1] != 2 || heap[2] != 1 || heap[3] != 0)
        return 0;
    heap[1] = 0;
    heap[3] = 2;
    depth[2] = 1;
    pqdownheap(depth, heap, 3, tree, 1);
    if (heap[1] != 1 || heap[2] != 0 || heap[3] != 2)
        return 0;
    heap[1] = 0;
    heap[2] = 1;
    tree[0].freq = 1;
    pqdownheap(depth, heap, 3, tree, 1);
    return heap[1] == 0 && heap[2] == 1 && heap[3] == 2;
}

static void init_histograms(void)
{
    const int span[CASES] = {2, 3, 19, 30, 64, 96, 160, 249,
                             286, 286, 286, 286, 286, 286, 286, 286};
    uint32_t rng = 0xb5d693e1u;
    for (int c = 0; c < CASES; ++c) {
        for (int n = 0; n < span[c]; ++n) {
            rng ^= rng << 13;
            rng ^= rng >> 17;
            rng ^= rng << 5;
            unsigned freq;
            switch (c % 4) {
            case 0: freq = 1; break; /* many ties */
            case 1: freq = 1 + (rng & 63u); break;
            case 2: freq = n < 4 ? 256 : 1 + (rng & 7u); break;
            default: freq = n < 2 || (rng & 3u) == 0 ? 1 + ((rng >> 8) & 31u) : 0; break;
            }
            /* Rotate symbol placement between cases without changing weights. */
            histograms[c][(n + 23 * c) % SYMBOLS].freq = (uint16_t)freq;
        }
    }
}

int main(void)
{
    node_t tree[HEAP_SIZE];
    init_histograms();
    if (!check_ties()) {
        fputs("Huffman heap tie-break mismatch\n", stderr);
        return 1;
    }
    for (int c = 0; c < CASES; ++c) {
        memcpy(tree, histograms[c], sizeof histograms[c]);
        int root = huffman_merge(tree);
        if (!check_tree(tree, root, histograms[c])) {
            fprintf(stderr, "Huffman tree mismatch: histogram %d\n", c);
            return 1;
        }
    }

    uint64_t checksum = UINT64_C(0xcbf29ce484222325);
    for (unsigned p = 0; p < PASSES; ++p) {
        for (int c = 0; c < CASES; ++c) {
            memcpy(tree, histograms[c], sizeof histograms[c]);
            int root = huffman_merge(tree);
            unsigned sample = (p + (unsigned)c * 43u) % SYMBOLS;
            uint64_t word = ((uint64_t)tree[root].freq << 32)
                          | ((uint64_t)tree[sample].dad << 16) | (unsigned)root;
            checksum = (checksum ^ word) * UINT64_C(0x100000001b3);
        }
    }
    printf("%016llx\n", (unsigned long long)checksum);
    return 0;
}
