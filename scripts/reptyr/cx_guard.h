#ifndef CX_GUARD_H
#define CX_GUARD_H
#include <sys/types.h>
// CX verifies this exact string before advertising adoption.
#define CX_REPTYR_VERSION "cx-reptyr 1 (reptyr 0.10.0)"
void cx_guard(int *argc, char **argv);
int cx_guard_recheck(pid_t pid);
void cx_drop_caps(void);
void cx_drop_privileges(void);
#endif
