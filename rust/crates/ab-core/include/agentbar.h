/* agentbar.h — AgentBar C ABI (scaffold).
 *
 * Stable surface for native hosts. Owned char* must be freed with ab_string_free.
 * Full lifecycle/snapshot API lands in later PRs; ab_version is the PR1 gate.
 *
 * Copyright (c) AgentBar contributors. MIT.
 */

#ifndef AGENTBAR_H
#define AGENTBAR_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Product / workspace version. Owned char*; free with ab_string_free. */
char *ab_version(void);

/* Free a string returned by any ab_* API. NULL is a no-op. */
void ab_string_free(char *s);

#ifdef __cplusplus
}
#endif

#endif /* AGENTBAR_H */
