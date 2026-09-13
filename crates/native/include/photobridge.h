#ifndef PHOTOBRIDGE_H
#define PHOTOBRIDGE_H
#ifdef __cplusplus
extern "C" {
#endif
// Blocking, thread-safe calls. Do not log JSON: pairing contains credentials.
char *photobridge_call(const char *request);
void photobridge_free(char *response);
#ifdef __cplusplus
}
#endif
#endif
