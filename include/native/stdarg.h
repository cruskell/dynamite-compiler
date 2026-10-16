#ifndef DYNAMITE_NATIVE_GNU_VA_LIST
#define DYNAMITE_NATIVE_GNU_VA_LIST
typedef __builtin_va_list __gnuc_va_list;
#endif
#if !defined(__need___va_list) && !defined(DYNAMITE_NATIVE_STDARG_H)
#define DYNAMITE_NATIVE_STDARG_H
typedef __builtin_va_list va_list;
#define va_start(ap, last) __builtin_va_start(ap, last)
#define va_arg(ap, type) __builtin_va_arg(ap, type)
#define va_copy(dest, src) __builtin_va_copy(dest, src)
#define va_end(ap) __builtin_va_end(ap)
#endif
#undef __need___va_list
