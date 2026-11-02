/* GCC-built diagnostic harness for the LCCC-compiled i686 fixture body. */
#define _GNU_SOURCE 1
#include <signal.h>
#include <sys/ucontext.h>
#include <unistd.h>

extern int vector_small_packed_wrappers_test_main(void);

static void append_hex(char *buffer, unsigned offset, unsigned long value)
{
    static const char digits[] = "0123456789abcdef";
    for (unsigned i = 0; i < 8; ++i)
        buffer[offset + i] = digits[(value >> (28 - 4 * i)) & 15];
}

static void report_fault(int sig, siginfo_t *info, void *context)
{
    const char prefix[] = "SIGSEGV EIP=0x00000000 ESP=0x00000000 EBP=0x00000000 ADDR=0x00000000\n";
    char message[sizeof(prefix) - 1];
    for (unsigned i = 0; i < sizeof(message); ++i)
        message[i] = prefix[i];
    const ucontext_t *uc = (const ucontext_t *)context;
    append_hex(message, 14, (unsigned long)uc->uc_mcontext.gregs[REG_EIP]);
    append_hex(message, 29, (unsigned long)uc->uc_mcontext.gregs[REG_ESP]);
    append_hex(message, 44, (unsigned long)uc->uc_mcontext.gregs[REG_EBP]);
    append_hex(message, 60, (unsigned long)info->si_addr);
    (void)write(STDERR_FILENO, message, sizeof(message));
    const char stack_prefix[] =
        "STACK=00000000,00000000,00000000,00000000,00000000\n";
    char stack_message[sizeof(stack_prefix) - 1];
    for (unsigned i = 0; i < sizeof(stack_message); ++i)
        stack_message[i] = stack_prefix[i];
    const unsigned long *sp =
        (const unsigned long *)uc->uc_mcontext.gregs[REG_ESP];
    for (unsigned i = 0; i < 5; ++i)
        append_hex(stack_message, 6 + 9 * i, sp[16 + i]);
    (void)write(STDERR_FILENO, stack_message, sizeof(stack_message));
    const char return_prefix[] = "RETADDR=0x00000000\n";
    char return_message[sizeof(return_prefix) - 1];
    for (unsigned i = 0; i < sizeof(return_message); ++i)
        return_message[i] = return_prefix[i];
    append_hex(return_message, 10, sp[15]);
    (void)write(STDERR_FILENO, return_message, sizeof(return_message));
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
