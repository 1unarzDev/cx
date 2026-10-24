// Synthetic "agent mid-turn": continuous tty output, worker threads, a piped
// child tool, stdin echo, and a signal/IO-error log for adoption fixtures.
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <pthread.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

static FILE *logf;
static volatile sig_atomic_t got_hup, got_winch, got_term, got_cont;

static double now(void) {
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec + t.tv_nsec / 1e9;
}
static void on_sig(int s) {
    if (s == SIGHUP) got_hup++;
    if (s == SIGWINCH) got_winch++;
    if (s == SIGTERM) got_term++;
    if (s == SIGCONT) got_cont++;
}
static void *spin(void *arg) {
    volatile unsigned long x = 0;
    for (;;) { x++; if (!(x & 0xfffff)) usleep(1000); }
    return arg;
}
int main(int argc, char **argv) {
    if (argc > 1 && !strcmp(argv[1], "--version")) { puts("churn 1"); return 0; }
    const char *path = getenv("CHURN_LOG");
    logf = fopen(path ? path : "churn.log", "a");
    if (!logf) return 2;
    setvbuf(logf, NULL, _IOLBF, 0);
    struct sigaction sa = {0};
    sa.sa_handler = on_sig;
    sigaction(SIGHUP, &sa, NULL);
    sigaction(SIGWINCH, &sa, NULL);
    sigaction(SIGTERM, &sa, NULL);
    sigaction(SIGCONT, &sa, NULL);
    signal(SIGPIPE, SIG_IGN);
    for (int i = 0; i < 4; i++) { pthread_t t; pthread_create(&t, NULL, spin, NULL); }
    int tool[2];
    if (pipe(tool)) return 3;
    pid_t child = fork();
    if (child == 0) {
        // A tool subprocess keeps default signal handling, unlike its agent.
        signal(SIGTERM, SIG_DFL);
        signal(SIGHUP, SIG_DFL);
        signal(SIGPIPE, SIG_DFL);
        close(tool[0]);
        for (unsigned long n = 0;; n++) { dprintf(tool[1], "tool %lu\n", n); usleep(20000); }
    }
    close(tool[1]);
    fcntl(tool[0], F_SETFL, O_NONBLOCK);
    fcntl(0, F_SETFL, fcntl(0, F_GETFL) | O_NONBLOCK);
    fprintf(logf, "start pid=%d child=%d\n", getpid(), child);
    double last = now(), worst = 0;
    unsigned long tick = 0, tool_lines = 0;
    char buf[512];
    for (;;) {
        tick++;
        int w = dprintf(1, "\rtick %lu ", tick);
        if (w < 0) fprintf(logf, "write-error errno=%d tick=%lu\n", errno, tick);
        ssize_t r = read(0, buf, sizeof buf - 1);
        if (r > 0) { buf[r] = 0; fprintf(logf, "input %zd %.*s\n", r, (int)strcspn(buf, "\r\n"), buf); dprintf(1, "[echo %.*s]", (int)r, buf); }
        else if (r == 0) fprintf(logf, "stdin-eof tick=%lu\n", tick);
        else if (errno != EAGAIN) fprintf(logf, "read-error errno=%d tick=%lu\n", errno, tick);
        while ((r = read(tool[0], buf, sizeof buf)) > 0) tool_lines++;
        if (got_hup) { fprintf(logf, "signal HUP\n"); got_hup = 0; }
        if (got_term) { fprintf(logf, "signal TERM\n"); return 0; }
        if (got_cont) { fprintf(logf, "signal CONT\n"); got_cont = 0; }
        if (got_winch) {
            struct winsize ws = {0};
            ioctl(1, TIOCGWINSZ, &ws);
            fprintf(logf, "signal WINCH %ux%u\n", ws.ws_col, ws.ws_row);
            got_winch = 0;
        }
        double t = now();
        if (t - last > worst) worst = t - last;
        if (!(tick % 50)) {
            char tty[256] = "";
            ssize_t n = readlink("/proc/self/fd/0", tty, sizeof tty - 1);
            if (n > 0) tty[n] = 0;
            fprintf(logf, "beat tick=%lu worst_gap_ms=%.0f tool=%lu tty=%s ppid=%d\n", tick, worst * 1000, tool_lines, tty, getppid());
            worst = 0;
        }
        last = t;
        if (waitpid(child, NULL, WNOHANG) == child) fprintf(logf, "tool-exited\n");
        usleep(10000);
    }
}
