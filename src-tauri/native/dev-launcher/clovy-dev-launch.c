// Development-only launcher for the Clovy dev app bundle (scripts/cargo-clovy-runner.sh).
//
// macOS attributes privacy requests (TCC) to the "responsible" process. A
// process spawned from a terminal inherits the terminal as its responsible
// process, so a dev app started by `pnpm tauri:dev` would use the terminal's
// Accessibility, Screen Recording, and Microphone grants instead of its own.
// This launcher replaces itself with the app (POSIX_SPAWN_SETEXEC keeps the
// pid, arguments, environment, stdio, and exit status that Tauri dev
// supervises) while disclaiming the terminal's responsibility, so macOS asks
// for and records each grant against the signed dev app bundle. It does not
// grant anything. Rationale and alternatives: docs/development.md.
//
// `responsibility_spawnattrs_setdisclaim` is exported by libSystem but is not
// public API. It is resolved at runtime; when it is missing the launch fails
// instead of silently inheriting the terminal's grants.

#include <dlfcn.h>
#include <errno.h>
#include <spawn.h>
#include <stdio.h>
#include <string.h>

extern char **environ;

typedef int (*disclaim_fn)(posix_spawnattr_t *attrs, int disclaim);

int main(int argc, char **argv) {
  if (argc < 2 || argv[1][0] != '/') {
    fprintf(stderr, "usage: %s /absolute/path/to/app/executable [arguments...]\n", argv[0]);
    return 64;
  }

  disclaim_fn disclaim =
      (disclaim_fn)dlsym(RTLD_DEFAULT, "responsibility_spawnattrs_setdisclaim");
  if (disclaim == NULL) {
    fprintf(stderr,
            "This macOS version does not provide responsibility_spawnattrs_setdisclaim, so the "
            "Clovy dev app cannot own its privacy permissions. See docs/development.md.\n");
    return 69;
  }

  posix_spawnattr_t attrs;
  int status = posix_spawnattr_init(&attrs);
  if (status == 0) status = posix_spawnattr_setflags(&attrs, POSIX_SPAWN_SETEXEC);
  if (status == 0) status = disclaim(&attrs, 1);
  if (status != 0) {
    fprintf(stderr, "Could not prepare the Clovy dev app launch: %s\n", strerror(status));
    return 71;
  }

  status = posix_spawn(NULL, argv[1], NULL, &attrs, &argv[1], environ);
  fprintf(stderr, "Could not start %s: %s\n", argv[1], strerror(status));
  posix_spawnattr_destroy(&attrs);
  return status == ENOENT ? 66 : 71;
}
