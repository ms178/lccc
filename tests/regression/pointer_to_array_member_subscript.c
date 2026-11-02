// Regression: a subscript of a pointer to a one-element array typedef is
// still an array lvalue.  It must decay to the address of the selected
// element for the following -> access, rather than loading the first word of
// the array as if it were a struct pointer.
struct mask {
    unsigned long bits[1];
};

typedef struct mask mask_var[1];

struct pod {
    mask_var *pod_cpus;
    int *cpu_pod;
};

static volatile unsigned long observed;

__attribute__((noinline))
static void consume(unsigned long *p) {
    observed = p[0];
}

__attribute__((noinline))
static void read_selected(struct pod *pt, int i) {
    consume(pt->pod_cpus[pt->cpu_pod[i]]->bits);
}

int main(void) {
    struct mask masks[3] = {{{11}}, {{22}}, {{33}}};
    int order[3] = {2, 0, 1};
    struct pod p = {(mask_var *)masks, order};

    read_selected(&p, 1);
    return observed != 11;
}
