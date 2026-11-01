/*
 * Diagnostic-only wrapper for vector_small_packed_wrappers_m32. It runs the
 * exact fixture body, with a signal handler that reports the faulting EIP on
 * i386. The fixture itself is included without edits; this wrapper is never a
 * correctness substitute for the ordinary regression.
 */
#define _GNU_SOURCE 1
#include <signal.h>
#include <sys/ucontext.h>
#include <unistd.h>

#define main vector_small_packed_wrappers_test_main
#include "vector_small_packed_wrappers_m32.c"
#undef main

static void report_fault(int sig, siginfo_t *info, void *context)
{
    (void)info;
    const char digits[] = "0123456789abcdef";
    const char prefix[] = "SIGSEGV EIP=0x00000000\n";
    char message[sizeof(prefix) - 1];
    for (unsigned i = 0; i < sizeof(message); ++i)
        message[i] = prefix[i];
    const ucontext_t *uc = (const ucontext_t *)context;
    unsigned long ip = (unsigned long)uc->uc_mcontext.gregs[REG_EIP];
    for (unsigned i = 0; i < 8; ++i)
        message[14 + i] = digits[(ip >> (28 - 4 * i)) & 15];
    (void)write(STDERR_FILENO, message, sizeof(message));
    _exit(128 + sig);
}

int main(void)
{
    struct sigaction action = {0};
    action.sa_sigaction = report_fault;
    action.sa_flags = SA_SIGINFO | SA_RESETHAND;
    sigemptyset(&action.sa_mask);
    if (sigaction(SIGSEGV, &action, 0) != 0)
        return 2;
    return vector_small_packed_wrappers_test_main();
}
