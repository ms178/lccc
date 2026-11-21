/* COMPANION harness, component half: this translation unit has no `main` and
 * is consumed by companion_link_main.c. The suite compiles it with `-c` and
 * requires that it DEFINES `companion_link_body_entry` (the .env sidecar says
 * so). A missing symbol or a wrong object class is a suite FAILURE, not a skip.
 *
 * The function body exercises a value that depends on the calling convention
 * (argument in, result out, a multiply and an add), so an ABI or lowering
 * defect shows up in the companion program's printed result. */
int companion_link_body_entry(int x)
{
    return x * 3 + 1;
}
