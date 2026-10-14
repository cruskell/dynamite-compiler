#ifndef DYNAMITE_STDLIB_H
#define DYNAMITE_STDLIB_H
/* exit, malloc, and free have builtin declarations in this C-like subset. */
#define EXIT_SUCCESS 0
#define EXIT_FAILURE 1
#include <stddef.h>
void *calloc(size_t count, size_t size);
void *realloc(void *pointer, size_t size);
int atoi(const char *text);
long atol(const char *text);
long long atoll(const char *text);
double atof(const char *text);
float strtof(const char *text, char **end);
double strtod(const char *text, char **end);
long strtol(const char *text, char **end, int base);
unsigned long strtoul(const char *text, char **end, int base);
long long strtoll(const char *text, char **end, int base);
unsigned long long strtoull(const char *text, char **end, int base);
#endif
