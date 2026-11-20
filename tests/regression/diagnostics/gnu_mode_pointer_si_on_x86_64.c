/* SI cannot represent the target pointer in 64-bit mode. */
typedef char *P __attribute__((mode(SI)));
P identity(P p) { return p; }
