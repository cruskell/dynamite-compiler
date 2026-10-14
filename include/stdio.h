#ifndef DYNAMITE_STDIO_H
#define DYNAMITE_STDIO_H
/* printf, puts, and putchar have builtin declarations in this C-like subset. */
#define EOF (-1)
#include <stddef.h>
int sprintf(char *buffer, const char *format, ...);
int snprintf(char *buffer, size_t size, const char *format, ...);
int sscanf(const char *text, const char *format, ...);
#endif
