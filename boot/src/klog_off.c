/* klog_off.c — 无日志版空桩:所有函数为空实现,不产生任何输出/内存开销。 */
#include "klog.h"
#include <stdarg.h>

void klog_init(void) {}
void klogf(int level, const char *fmt, ...) { (void)level; (void)fmt; }
void kerr(int code, const char *what, unsigned long a, unsigned long b,
          unsigned long c) {
    (void)code;
    (void)what;
    (void)a;
    (void)b;
    (void)c;
}
void klog_dump_ring(void) {}