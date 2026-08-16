#ifndef MCS_KLOG_H
#define MCS_KLOG_H

/* 内核日志:C 实现,COM1 + 内存环形缓冲。
   级别:0=ERR 1=WARN 2=INFO 3=DBG。
   支持格式:%s %c %d %u %x %X %lx %llx %p %% */

enum {
    KLOG_ERR = 0,
    KLOG_WARN = 1,
    KLOG_INFO = 2,
    KLOG_DBG = 3,
};

void klog_init(void);
void klogf(int level, const char *fmt, ...);
void kerr(int code, const char *what, unsigned long a, unsigned long b, unsigned long c);
void klog_dump_ring(void);

#endif