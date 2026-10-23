#ifndef REBASE_LINK_H
#define REBASE_LINK_H
#ifdef __cplusplus
extern "C" {
#endif
// Returns owned UTF-8 JSON. Every non-null result must be released once.
char *rebase_link_call(const char *input);
void rebase_link_free(char *output);
#ifdef __cplusplus
}
#endif
#endif
