/* The negative controls of check_x86_gpr_intrinsics.py: three callers the
 * two compilers are made to compile differently (the gate passes
 * -DCONTROL_BROKEN to lccc only), each wrong in one of the ways the gate
 * exists to catch, and one that holds no intrinsic at all. All four have
 * to be reported, in these words. */

/* A caller whose code is register and memory moves only has nothing the
 * gate could compare or step over. That is what a caller looks like when
 * its intrinsic's instruction is taken for glue (MOVDIRI was): it must be
 * a failure, not a caller silently left out. */
void t_control_glue(unsigned *line, unsigned value)
{
    *line = value;
}

void t_control_implicit(unsigned key)
{
#ifdef CONTROL_BROKEN
    /* WRPKRU faults unless ECX is 0. */
    __asm__ __volatile__("wrpkru" : : "a"(key), "c"(1), "d"(0));
#else
    __asm__ __volatile__("wrpkru" : : "a"(key), "c"(0), "d"(0));
#endif
}

void t_control_operand(void *line, void *other)
{
#ifdef CONTROL_BROKEN
    /* The wrong line is written back. */
    __asm__ __volatile__("clwb %0" : : "m"(*(volatile char *)other));
    (void)line;
#else
    __asm__ __volatile__("clwb %0" : : "m"(*(volatile char *)line));
    (void)other;
#endif
}

unsigned char t_control_form(unsigned control, unsigned long long deadline)
{
    unsigned char result;
#ifdef CONTROL_BROKEN
    /* TPAUSE reports through CF, not ZF. */
    __asm__ __volatile__("tpause %1\n\tsetz %0"
#else
    __asm__ __volatile__("tpause %1\n\tsetc %0"
#endif
                         : "=q"(result)
                         : "r"(control), "a"((unsigned)deadline), "d"((unsigned)(deadline >> 32))
                         : "cc");
    return result;
}
