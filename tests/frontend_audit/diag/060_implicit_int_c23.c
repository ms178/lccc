/* expect: reject */
/* C23 6.9.1p2 removed the implicit-int rule, and GCC 16.2 rejects it in every
   -std mode (measured: c89..c23 all rc=1), so does lccc. The earlier corpus
   entry claimed acceptance and was simply wrong. */
main() { return 0; }
