#include <stdint.h>

/* Regression test for Linux 6.18.58 `__rb_insert_augmented` (`lib/rbtree.c`)
 * boot panic: a loop-carried pointer web (`parent`) coalesced with an entry
 * copy and was consumed as arg0 of an indirect callback (`augment_rotate`) in
 * a function containing `asm volatile("" ::: "memory")` (`WRITE_ONCE`, forcing
 * the mature `CallIndirect` codegen path). Before propagating
 * `indirect_arg_values` and `later_arg_values` across copy-coalesce webs (and
 * adding `%r10` defense-in-depth in `emit_call_spill_fptr_with_args_impl`),
 * `parent` received `%r10` and `movq -32(%rbp), %r10` overwrote `parent` with
 * the callback function pointer before `movq %r10, %rdi`. */

struct aug_node {
    uintptr_t parent_color;
    struct aug_node *right;
    struct aug_node *left;
    uint64_t val;
    uint64_t subtree_max;
};

#define RB_RED   0UL
#define RB_BLACK 1UL
#define rb_parent(r) ((struct aug_node *)((r)->parent_color & ~3UL))

#define WRITE_ONCE_PTR(dst, v) do { \
    *(volatile struct aug_node **)&(dst) = (v); \
    __asm__ __volatile__("" ::: "memory"); \
} while (0)

static uint64_t compute_max(struct aug_node *n) {
    uint64_t m = n->val;
    if (n->left && n->left->subtree_max > m)
        m = n->left->subtree_max;
    if (n->right && n->right->subtree_max > m)
        m = n->right->subtree_max;
    return m;
}

__attribute__((noinline))
static void aug_rotate_cb(struct aug_node *old, struct aug_node *new_node) {
    if (!old || !new_node)
        __builtin_trap();
    /* Catch passing the function pointer itself instead of `old`. */
    if ((uintptr_t)old == (uintptr_t)&aug_rotate_cb)
        __builtin_trap();
    new_node->subtree_max = old->subtree_max;
    old->subtree_max = compute_max(old);
}

__attribute__((noinline))
static void rb_insert_augmented_repro(
    struct aug_node *node,
    struct aug_node **root,
    void (*augment_rotate)(struct aug_node *old, struct aug_node *new_node))
{
    struct aug_node *parent = (struct aug_node *)node->parent_color;
    struct aug_node *gparent, *tmp;

    for (;;) {
        if (!parent) {
            node->parent_color = RB_BLACK;
            break;
        }
        if (parent->parent_color & RB_BLACK)
            break;

        gparent = rb_parent(parent);
        tmp = gparent->right;
        if (parent != tmp) {
            if (tmp && !(tmp->parent_color & RB_BLACK)) {
                tmp->parent_color = (uintptr_t)gparent | RB_BLACK;
                parent->parent_color = (uintptr_t)gparent | RB_BLACK;
                node = gparent;
                parent = rb_parent(node);
                node->parent_color = (uintptr_t)parent | RB_RED;
                continue;
            }
            tmp = parent->right;
            if (node == tmp) {
                tmp = node->left;
                WRITE_ONCE_PTR(parent->right, tmp);
                WRITE_ONCE_PTR(node->left, parent);
                if (tmp)
                    tmp->parent_color = (uintptr_t)parent | RB_BLACK;
                parent->parent_color = (uintptr_t)node | RB_RED;
                augment_rotate(parent, node);
                parent = node;
                tmp = node->right;
            }
            WRITE_ONCE_PTR(gparent->left, tmp);
            WRITE_ONCE_PTR(parent->right, gparent);
            if (tmp)
                tmp->parent_color = (uintptr_t)gparent | RB_BLACK;
            tmp = rb_parent(gparent);
            parent->parent_color = gparent->parent_color;
            gparent->parent_color = (uintptr_t)parent | RB_RED;
            if (!tmp)
                *root = parent;
            else if (tmp->left == gparent)
                WRITE_ONCE_PTR(tmp->left, parent);
            else
                WRITE_ONCE_PTR(tmp->right, parent);
            augment_rotate(gparent, parent);
            break;
        } else {
            tmp = gparent->left;
            if (tmp && !(tmp->parent_color & RB_BLACK)) {
                tmp->parent_color = (uintptr_t)gparent | RB_BLACK;
                parent->parent_color = (uintptr_t)gparent | RB_BLACK;
                node = gparent;
                parent = rb_parent(node);
                node->parent_color = (uintptr_t)parent | RB_RED;
                continue;
            }
            tmp = parent->left;
            if (node == tmp) {
                tmp = node->right;
                WRITE_ONCE_PTR(parent->left, tmp);
                WRITE_ONCE_PTR(node->right, parent);
                if (tmp)
                    tmp->parent_color = (uintptr_t)parent | RB_BLACK;
                parent->parent_color = (uintptr_t)node | RB_RED;
                augment_rotate(parent, node);
                parent = node;
                tmp = node->left;
            }
            WRITE_ONCE_PTR(gparent->right, tmp);
            WRITE_ONCE_PTR(parent->left, gparent);
            if (tmp)
                tmp->parent_color = (uintptr_t)gparent | RB_BLACK;
            tmp = rb_parent(gparent);
            parent->parent_color = gparent->parent_color;
            gparent->parent_color = (uintptr_t)parent | RB_RED;
            if (!tmp)
                *root = parent;
            else if (tmp->right == gparent)
                WRITE_ONCE_PTR(tmp->right, parent);
            else
                WRITE_ONCE_PTR(tmp->left, parent);
            augment_rotate(gparent, parent);
            break;
        }
    }
}

int main(void) {
    /* Case 1: Left-Right double rotation (exercises `.LBB209` `augment_rotate(parent, node)`). */
    struct aug_node g1 = { RB_BLACK, 0, 0, 30, 30 };
    struct aug_node p1 = { (uintptr_t)&g1 | RB_RED, 0, 0, 10, 20 };
    struct aug_node n1 = { (uintptr_t)&p1 | RB_RED, 0, 0, 20, 20 };
    g1.left = &p1;
    p1.right = &n1;
    g1.subtree_max = 30;
    struct aug_node *root1 = &g1;
    rb_insert_augmented_repro(&n1, &root1, aug_rotate_cb);
    if (root1 != &n1 || n1.left != &p1 || n1.right != &g1)
        return 1;
    if (p1.subtree_max != 10 || g1.subtree_max != 30 || n1.subtree_max != 30)
        return 2;

    /* Case 2: Right-Left double rotation (exercises `.LBB222` `augment_rotate(parent, node)`). */
    struct aug_node g2 = { RB_BLACK, 0, 0, 10, 30 };
    struct aug_node p2 = { (uintptr_t)&g2 | RB_RED, 0, 0, 30, 30 };
    struct aug_node n2 = { (uintptr_t)&p2 | RB_RED, 0, 0, 20, 20 };
    g2.right = &p2;
    p2.left = &n2;
    struct aug_node *root2 = &g2;
    rb_insert_augmented_repro(&n2, &root2, aug_rotate_cb);
    if (root2 != &n2 || n2.left != &g2 || n2.right != &p2)
        return 3;
    if (g2.subtree_max != 10 || p2.subtree_max != 30 || n2.subtree_max != 30)
        return 4;

    return 0;
}
