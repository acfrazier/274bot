/*
 * nightly274-launch: the launchd entry point of the revision-274 pull on macOS.
 *
 * macOS privacy control (TCC) judges file access by a launchd job by the
 * job's own program. /bin/bash started by launchd cannot write the external
 * volume that holds the mirror, so the pull's mkdir of its lock failed every
 * hour ("Operation not permitted") and the script blamed another pull. Grant
 * this small binary, not /bin/bash, access to removable volumes (macOS
 * prompts for it at the first run; Full Disk Access also works). It starts
 * the pull as a child process (posix_spawn, not exec), so this binary stays
 * the responsible process for the whole run.
 *
 * It runs one fixed command only:
 *   /usr/bin/caffeinate -i /bin/bash $HOME/.local/lib/274bot-nightly/pull-274.sh
 * caffeinate keeps the Mac awake for the duration of the pull.
 *
 * Build (on the Mac):
 *   clang -O2 -Wall -o ~/.local/libexec/274bot-nightly/nightly274-launch nightly274-launch.c
 * A rebuild changes the binary's code hash, so the grant must be renewed.
 */
#include <errno.h>
#include <spawn.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/wait.h>

extern char **environ;

int main(void)
{
	const char *home = getenv("HOME");
	if (home == NULL || home[0] != '/') {
		fprintf(stderr, "nightly274-launch: HOME is not set\n");
		return 78;
	}
	char script[1024];
	int n = snprintf(script, sizeof script, "%s/.local/lib/274bot-nightly/pull-274.sh", home);
	if (n < 0 || (size_t)n >= sizeof script) {
		fprintf(stderr, "nightly274-launch: HOME is too long\n");
		return 78;
	}
	char *args[] = { "/usr/bin/caffeinate", "-i", "/bin/bash", script, NULL };
	pid_t pid;
	int rc = posix_spawn(&pid, args[0], NULL, NULL, args, environ);
	if (rc != 0) {
		fprintf(stderr, "nightly274-launch: posix_spawn: %s\n", strerror(rc));
		return 71;
	}
	int status;
	while (waitpid(pid, &status, 0) < 0) {
		if (errno != EINTR) {
			fprintf(stderr, "nightly274-launch: waitpid: %s\n", strerror(errno));
			return 71;
		}
	}
	if (WIFEXITED(status))
		return WEXITSTATUS(status);
	return 128 + WTERMSIG(status);
}
