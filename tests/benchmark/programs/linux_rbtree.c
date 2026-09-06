/*
 * Workload-derived kernel: Linux kernel lib/rbtree.c (GPL-2.0-or-later).
 *
 * Extracted from Linux kernel rbtree implementation (tools/lib/rbtree.c).
 * Red-Black trees are the fundamental data structure in the Linux CFS
 * scheduler, memory management (vma tree), timer wheel, and epoll.
 *
 * This kernel stresses pointer-tagging (color in low bits of parent pointer),
 * branch prediction, rotation/rebalancing logic, and memory layout.
 */
#include <stdio.h>

#define NODE_COUNT 16384U
#define LOOKUP_ROUNDS 8U

typedef unsigned long uintptr;
typedef unsigned long long u64;

#define RB_RED   0
#define RB_BLACK 1

struct rb_node {
  unsigned long __rb_parent_color;
  struct rb_node *rb_right;
  struct rb_node *rb_left;
  int key;
  int val;
};

struct rb_root {
  struct rb_node *rb_node;
};

#define rb_parent(r)   ((struct rb_node *)((r)->__rb_parent_color & ~3UL))
#define rb_color(r)    ((r)->__rb_parent_color & 1UL)
#define rb_is_red(r)   (!rb_color(r))
#define rb_is_black(r) (rb_color(r))
#define rb_set_red(r)  do { (r)->__rb_parent_color &= ~1UL; } while (0)
#define rb_set_black(r) do { (r)->__rb_parent_color |= 1UL; } while (0)

static inline void
rb_set_parent(struct rb_node *rb, struct rb_node *p)
{
  rb->__rb_parent_color = (rb->__rb_parent_color & 3UL) | (unsigned long)p;
}

static inline void
rb_set_parent_color(struct rb_node *rb, struct rb_node *p, int color)
{
  rb->__rb_parent_color = (unsigned long)p | (unsigned long)color;
}

static void
__rb_change_child(struct rb_node *old, struct rb_node *new_node,
                  struct rb_node *parent, struct rb_root *root)
{
  if (parent) {
    if (parent->rb_left == old)
      parent->rb_left = new_node;
    else
      parent->rb_right = new_node;
  } else {
    root->rb_node = new_node;
  }
}

static void
__rb_rotate_set_parents(struct rb_node *old, struct rb_node *new_node,
                        struct rb_root *root, int color)
{
  struct rb_node *parent = rb_parent(old);
  new_node->__rb_parent_color = old->__rb_parent_color;
  rb_set_parent_color(old, new_node, color);
  __rb_change_child(old, new_node, parent, root);
}

static void
rb_insert_color(struct rb_node *node, struct rb_root *root)
{
  struct rb_node *parent = rb_parent(node), *gparent, *tmp;

  while (1) {
    if (!parent) {
      rb_set_black(node);
      break;
    }
    if (rb_is_black(parent))
      break;

    gparent = rb_parent(parent);
    tmp = gparent->rb_right;

    if (parent != tmp) {
      if (tmp && rb_is_red(tmp)) {
        rb_set_black(tmp);
        rb_set_black(parent);
        rb_set_red(gparent);
        node = gparent;
        parent = rb_parent(node);
        continue;
      }

      tmp = parent->rb_right;
      if (node == tmp) {
        tmp = node->rb_left;
        parent->rb_right = tmp;
        node->rb_left = parent;
        if (tmp)
          rb_set_parent_color(tmp, parent, RB_BLACK);
        rb_set_parent_color(parent, node, RB_RED);
        parent = node;
        tmp = node->rb_right;
      }

      gparent->rb_left = tmp;
      parent->rb_right = gparent;
      if (tmp)
        rb_set_parent_color(tmp, gparent, RB_BLACK);
      __rb_rotate_set_parents(gparent, parent, root, RB_RED);
      break;
    } else {
      tmp = gparent->rb_left;
      if (tmp && rb_is_red(tmp)) {
        rb_set_black(tmp);
        rb_set_black(parent);
        rb_set_red(gparent);
        node = gparent;
        parent = rb_parent(node);
        continue;
      }

      tmp = parent->rb_left;
      if (node == tmp) {
        tmp = node->rb_right;
        parent->rb_left = tmp;
        node->rb_right = parent;
        if (tmp)
          rb_set_parent_color(tmp, parent, RB_BLACK);
        rb_set_parent_color(parent, node, RB_RED);
        parent = node;
        tmp = node->rb_left;
      }

      gparent->rb_right = tmp;
      parent->rb_left = gparent;
      if (tmp)
        rb_set_parent_color(tmp, gparent, RB_BLACK);
      __rb_rotate_set_parents(gparent, parent, root, RB_RED);
      break;
    }
  }
}

static struct rb_node *
rb_search(struct rb_root *root, int key)
{
  struct rb_node *node = root->rb_node;
  while (node) {
    if (key < node->key)
      node = node->rb_left;
    else if (key > node->key)
      node = node->rb_right;
    else
      return node;
  }
  return NULL;
}

static struct rb_node node_pool[NODE_COUNT];

int
main(void)
{
  struct rb_root root = { NULL };
  unsigned int i, r;
  u64 checksum = 0U;
  unsigned int state = 0x4d2b79f5U;

  for (i = 0; i < NODE_COUNT; i++) {
    struct rb_node **new_slot = &(root.rb_node), *parent = NULL;
    state = state * 1664525U + 1013904223U;
    node_pool[i].key = (int)(state & 0x7fffffffU);
    node_pool[i].val = (int)i;
    node_pool[i].rb_left = NULL;
    node_pool[i].rb_right = NULL;

    while (*new_slot) {
      parent = *new_slot;
      if (node_pool[i].key < parent->key)
        new_slot = &((*new_slot)->rb_left);
      else
        new_slot = &((*new_slot)->rb_right);
    }

    rb_set_parent_color(&node_pool[i], parent, RB_RED);
    *new_slot = &node_pool[i];
    rb_insert_color(&node_pool[i], &root);
  }

  for (r = 0; r < LOOKUP_ROUNDS; r++) {
    for (i = 0; i < NODE_COUNT; i++) {
      int query = node_pool[(i * 104729U) % NODE_COUNT].key;
      struct rb_node *found = rb_search(&root, query);
      if (found)
        checksum += (u64)found->val ^ ((u64)found->key << 16);
    }
  }

  printf("%016llx\n", checksum);
  return 0;
}
