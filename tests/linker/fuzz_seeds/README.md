# Fuzzer seed corpus

`good.c` builds a **freestanding, fully self-contained** executable: no libc,
no dynamic loader, and it runs and returns a checkable exit status.

That property is the whole point. An earlier seed used `printf`, so every
mutant died at symbol resolution with "undefined reference to printf" and the
campaign never reached layout, relocation or emission — 250 mutants found
nothing. With this seed the same fuzzer found two real crashes within minutes
(an integer-overflow bounds check and an unbounded allocation).

Regenerate the binary seeds with:

    ./generate_seeds.sh
