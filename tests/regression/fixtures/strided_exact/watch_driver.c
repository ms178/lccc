/* Hardware read watchpoint on a gap between live scalar elements. The
   positive control proves that ptrace/debug registers actually fired. */
#include <errno.h>
#include <signal.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/ptrace.h>
#include <sys/types.h>
#include <sys/user.h>
#include <sys/wait.h>
#include <unistd.h>

uint32_t strided_watch_kernel(const unsigned char *p);
static uint32_t words[256];

static int run_watch(int control, uint32_t expected) {
    pid_t pid = fork();
    if (pid < 0) { perror("fork"); return 1; }
    if (pid == 0) {
        if (ptrace(PTRACE_TRACEME, 0, 0, 0) == -1) _exit(80);
        raise(SIGSTOP);
        if (control) {
            (void)*(volatile uint32_t *)&words[1];
            _exit(0);
        }
        _exit(strided_watch_kernel((const unsigned char *)words) == expected ? 0 : 82);
    }
    int status = 0;
    if (waitpid(pid, &status, 0) != pid || !WIFSTOPPED(status) || WSTOPSIG(status) != SIGSTOP) {
        fprintf(stderr, "watchpoint child did not stop: %#x\n", status);
        return 2;
    }
    /* DR0 = gap address; enable local breakpoint 0 with RW=read/write,
       LEN=4 bytes. The address is 4-byte aligned. */
    if (ptrace(PTRACE_POKEUSER, pid, offsetof(struct user, u_debugreg[0]),
               (unsigned long)&words[1]) == -1 ||
        ptrace(PTRACE_POKEUSER, pid, offsetof(struct user, u_debugreg[7]),
               (unsigned long)0xF0001) == -1) {
        perror("setting read watchpoint");
        kill(pid, SIGKILL);
        waitpid(pid, &status, 0);
        return 3;
    }
    int hits = 0;
    for (;;) {
        if (ptrace(PTRACE_CONT, pid, 0, 0) == -1) { perror("continue"); return 4; }
        if (waitpid(pid, &status, 0) != pid) { perror("waitpid"); return 5; }
        if (WIFEXITED(status)) {
            if (WEXITSTATUS(status) != 0) {
                fprintf(stderr, "watchpoint child exited %d\n", WEXITSTATUS(status));
                return 6;
            }
            break;
        }
        if (!WIFSTOPPED(status) || WSTOPSIG(status) != SIGTRAP) {
            fprintf(stderr, "unexpected watchpoint status %#x\n", status);
            kill(pid, SIGKILL);
            waitpid(pid, &status, 0);
            return 7;
        }
        ++hits;
    }
    if (control ? hits != 1 : hits != 0) {
        fprintf(stderr, "%s watchpoint hits: %d\n", control ? "control" : "strided kernel", hits);
        return 8;
    }
    return 0;
}

int main(void) {
    uint32_t expected = 0;
    for (unsigned i = 0; i < 256; ++i) words[i] = i * 2654435761u + 17u;
    for (unsigned i = 0; i < 128; ++i) expected += words[2 * i] ^ (8 * i);
    if (run_watch(1, expected) || run_watch(0, expected)) return 1;
    puts("strided exact-access watchpoint: PASS");
    return 0;
}
