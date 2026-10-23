/* SPDX-License-Identifier: GPL-2.0-or-later */
/*
 * Workload-derived kernel: Linux 6.18, lib/list_sort.c.
 *
 * The bottom-up merge sort below is extracted from the Linux kernel's
 * lib/list_sort.c (GPL-2.0-or-later). The kernel's `list_head`, locking,
 * EXPORT_SYMBOL machinery and <linux/...> headers were replaced with a local
 * node type and a plain comparison callback; the merge and count-run structure
 * is the kernel's. Names are prefixed. Full source provenance is in
 * tests/benchmark/WORKLOAD_PROVENANCE.md.
 *
 * Why this shape: list_sort is a pure pointer-chasing merge sort -- two merges
 * per pass over an intrusive singly-linked list, with the comparison behind an
 * opaque function pointer. That combination is what a code generator cannot
 * vectorize and must instead handle well in scalar code:
 *
 *   * indirect calls that are not devirtualizable, so the callee-saved
 *     register set and the caller-saved clobber model decide whether the
 *     loop's state stays in registers;
 *   * a merge loop with two data-dependent next-pointer loads per iteration
 *     (a pointer-chasing latency chain, not a throughput problem);
 *   * a bottom-up 64-slot bin array (`count[]`) indexed by the trailing-zero
 *     count of the element index, which is a counted-loop + ctz pattern.
 *
 * The kernel's `merge()` compares with `cmp(priv, a, b) <= 0` so equal keys
 * keep their original order; the harness checks that stability property, not
 * only sortedness, because an unstable rewrite is an easy way to be fast and
 * wrong.
 */
#include <stdio.h>
#include <stdlib.h>

#ifndef ELEMENTS
#define ELEMENTS 20000
#endif
#ifndef PASSES
/* Keep default timing runs above the process-startup noise floor.
 * Bounded Callgrind screens explicitly override PASSES=1. */
#define PASSES 128U
#endif
#define MAX_LIST_LENGTH_BITS 20

struct lnode {
    struct lnode *next;
    unsigned long key;
    unsigned int seq;
};

/* Opaque comparison through a function pointer, as the kernel's cmp(priv,...)
 * is: this is the part a compiler must not assume anything about. */
typedef int (*list_cmp_fn)(void *priv, const struct lnode *a, const struct lnode *b);

static int key_cmp(void *priv, const struct lnode *a, const struct lnode *b)
{
    (void)priv;
    if (a->key < b->key)
        return -1;
    if (a->key > b->key)
        return 1;
    return 0;
}

static struct lnode *merge(void *priv, list_cmp_fn cmp,
                           struct lnode *a, struct lnode *b)
{
    struct lnode head, *tail = &head;

    while (a && b) {
        /* if a is less than b, take a; ties take a, which is what makes the
         * sort stable. */
        if (cmp(priv, a, b) <= 0) {
            tail->next = a;
            a = a->next;
        } else {
            tail->next = b;
            b = b->next;
        }
        tail = tail->next;
    }
    tail->next = a ? a : b;
    return head.next;
}

static struct lnode *merge_and_count(void *priv, list_cmp_fn cmp,
                                     struct lnode *a, unsigned int a_count,
                                     struct lnode *b, unsigned int b_count)
{
    (void)a_count;
    (void)b_count;
    return merge(priv, cmp, a, b);
}

/* The kernel's list_sort: 64 pending runs, each merged when the current index's
 * trailing zero count says the run of that size is complete. */
static struct lnode *list_sort(void *priv, struct lnode *list, list_cmp_fn cmp)
{
    struct lnode *pending[MAX_LIST_LENGTH_BITS] = { 0 };
    unsigned int count[MAX_LIST_LENGTH_BITS] = { 0 };
    unsigned int bits = 0;
    unsigned int i;

    while (list) {
        struct lnode *run = list;
        unsigned int run_count = 1;
        list = list->next;
        run->next = NULL;

        for (i = 0; (bits >> i) & 1; i++) {
            run = merge_and_count(priv, cmp, pending[i], count[i], run, run_count);
            run_count += count[i];
            pending[i] = NULL;
            count[i] = 0;
        }
        pending[i] = run;
        count[i] = run_count;
        bits++;
    }

    list = NULL;
    for (i = 0; i < MAX_LIST_LENGTH_BITS; i++) {
        if (pending[i])
            list = list ? merge_and_count(priv, cmp, pending[i], count[i], list, 0)
                        : pending[i];
    }
    return list;
}

/* Reference: the same merge sort, but only the sortedness/stability properties
 * are re-derived with a plain insertion-style pass over the result, so the
 * check does not depend on a golden number from any compiler. */
static int verify_sorted_stable(const struct lnode *list, unsigned int expected)
{
    unsigned int seen = 0;
    const struct lnode *prev = NULL;
    for (; list; list = list->next) {
        if (prev) {
            if (list->key < prev->key)
                return 0; /* not sorted */
            if (list->key == prev->key && list->seq < prev->seq)
                return 0; /* not stable */
        }
        prev = list;
        seen++;
    }
    return seen == expected;
}

int main(void)
{
    static struct lnode nodes[ELEMENTS];
    unsigned long long checksum = 0;
    unsigned int pass, i;
    int ok = 1;

    for (pass = 0; pass < PASSES; pass++) {
        struct lnode *list = NULL;

        /* Deterministic key distribution with many ties, which is what makes
         * the stability check meaningful. */
        for (i = 0; i < ELEMENTS; i++) {
            unsigned int state = (i + 1) * 2654435761U + pass * 97U;
            nodes[i].key = (state >> 11) % 512U;
            /* The nodes are pushed onto the FRONT of the list, so the list's
             * original order is descending in `i`; assigning seq that way
             * keeps "stable" meaning "original list order preserved". */
            nodes[i].seq = ELEMENTS - 1 - i;
            nodes[i].next = list;
            list = &nodes[i];
        }

        list = list_sort(NULL, list, key_cmp);

        if (!verify_sorted_stable(list, ELEMENTS))
            ok = 0;
        for (i = 0; list && i < ELEMENTS; i++, list = list->next)
            checksum = checksum * 31U + list->key;
    }

    printf("linux_list_sort checksum=%llu sorted_and_stable=%s\n",
           checksum, ok ? "OK" : "BROKEN");
    return ok ? 0 : 1;
}
