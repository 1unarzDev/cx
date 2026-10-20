// CX guard for a capability-enabled reptyr. The binary may carry
// cap_sys_ptrace, so it accepts exactly one form:
//
//   cx-reptyr -T TARGET TARGET_START LEADER LEADER_START HOLDER HOLDER_START
//
// Every process it may trace must belong entirely to the caller's real
// uid/gid, still have the start time CX observed, and form the expected
// terminal session: TARGET in LEADER's session on a Unix98 pty, and HOLDER the
// leader's parent (reptyr's terminal emulator). Identities are re-checked
// after each ptrace attach, and every capability is dropped before relaying.
#include <errno.h>
#include <linux/capability.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <unistd.h>

#include "cx_guard.h"

struct cx_allowed {
    pid_t pid;
    unsigned long long start;
};
static struct cx_allowed allowed[3];

struct cx_stat {
    pid_t ppid, session;
    int tty;
    unsigned long long start;
};

static int read_stat(pid_t pid, struct cx_stat *out) {
    char path[64], buf[1024];
    snprintf(path, sizeof path, "/proc/%d/stat", (int)pid);
    FILE *f = fopen(path, "re");
    if (!f)
        return -1;
    size_t n = fread(buf, 1, sizeof buf - 1, f);
    fclose(f);
    buf[n] = 0;
    char *p = strrchr(buf, ')');
    if (!p)
        return -1;
    char state;
    int pgrp, tpgid;
    unsigned long long skip;
    // Fields 3..22 of proc(5): state ppid pgrp session tty_nr tpgid ... starttime.
    if (sscanf(p + 2, "%c %d %d %d %d %d %llu %llu %llu %llu %llu %llu %llu %llu %llu %llu %llu %llu %llu %llu",
               &state, &out->ppid, &pgrp, &out->session, &out->tty, &tpgid,
               &skip, &skip, &skip, &skip, &skip, &skip, &skip, &skip, &skip,
               &skip, &skip, &skip, &skip, &out->start) != 20)
        return -1;
    return 0;
}

// All real, effective, saved and filesystem ids must be the caller's. The
// /proc directory owner also changes to root for non-dumpable processes.
static int same_owner(pid_t pid) {
    char path[64], line[256];
    struct stat st;
    snprintf(path, sizeof path, "/proc/%d", (int)pid);
    if (stat(path, &st) || st.st_uid != getuid() || st.st_gid != getgid())
        return 0;
    snprintf(path, sizeof path, "/proc/%d/status", (int)pid);
    FILE *f = fopen(path, "re");
    if (!f)
        return 0;
    int uid_ok = 0, gid_ok = 0;
    while (fgets(line, sizeof line, f)) {
        unsigned a, b, c, d;
        if (sscanf(line, "Uid: %u %u %u %u", &a, &b, &c, &d) == 4)
            uid_ok = a == getuid() && b == getuid() && c == getuid() && d == getuid();
        if (sscanf(line, "Gid: %u %u %u %u", &a, &b, &c, &d) == 4)
            gid_ok = a == getgid() && b == getgid() && c == getgid() && d == getgid();
    }
    fclose(f);
    return uid_ok && gid_ok;
}

static int parse(const char *s, unsigned long long *out) {
    char *end = NULL;
    errno = 0;
    unsigned long long v = strtoull(s, &end, 10);
    if (errno || !*s || *end || v == 0)
        return -1;
    *out = v;
    return 0;
}

static int verify(pid_t pid, unsigned long long start, struct cx_stat *st) {
    return read_stat(pid, st) == 0 && st->start == start && same_owner(pid);
}

static void refuse(const char *why) {
    fprintf(stderr, "cx-reptyr: refused: %s\n", why);
    exit(77);
}

void cx_guard(int *argc, char **argv) {
    if (*argc == 2 && !strcmp(argv[1], "-v")) {
        puts(CX_REPTYR_VERSION);
        exit(0);
    }
    if (*argc != 8 || strcmp(argv[1], "-T"))
        refuse("usage: cx-reptyr -T TARGET START LEADER START HOLDER START");
    unsigned long long v[6];
    for (int i = 0; i < 6; i++)
        if (parse(argv[2 + i], &v[i]) || (i % 2 == 0 && v[i] > 0x3fffffff))
            refuse("invalid identity");
    struct cx_stat target, leader, holder;
    if (!verify((pid_t)v[0], v[1], &target) || !verify((pid_t)v[2], v[3], &leader) ||
        !verify((pid_t)v[4], v[5], &holder))
        refuse("process identity changed or is not owned by the caller");
    int major = ((unsigned)target.tty >> 8) & 0xfff;
    if (major < 136 || major > 143)
        refuse("target is not on a pseudo-terminal");
    if (target.session != (pid_t)v[2] || leader.session != (pid_t)v[2] || leader.tty != target.tty)
        refuse("target is not in the expected terminal session");
    if (leader.ppid != (pid_t)v[4])
        refuse("terminal holder is not the session leader's parent");
    for (int i = 0; i < 3; i++) {
        allowed[i].pid = (pid_t)v[2 * i];
        allowed[i].start = v[2 * i + 1];
    }
    argv[3] = NULL;
    *argc = 3;
}

int cx_guard_recheck(pid_t pid) {
    struct cx_stat st;
    for (int i = 0; i < 3; i++)
        if (allowed[i].pid == pid)
            return verify(pid, allowed[i].start, &st) ? 0 : -1;
    return -1;
}

void cx_drop_caps(void) {
    struct __user_cap_header_struct header = {_LINUX_CAPABILITY_VERSION_3, 0};
    struct __user_cap_data_struct data[2] = {{0}};
    if (syscall(SYS_capset, &header, data)) {
        perror("cx-reptyr: capset");
        exit(1);
    }
}
