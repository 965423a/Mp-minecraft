/* klog.c — 内核日志:COM1 串口 + 内存环形缓冲。
   错误根因记录:kerr() 把 错误码/说明/关键值(地址、寄存器、IDT/LVT 状态)
   打进串口与环形缓冲;崩溃后可用 QEMU monitor `xp` 读 KLOG_RING 直接看根因。 */

#include "klog.h"
#include <stdarg.h>

#define COM1 0x3F8
#define COM1_LSR 0x3FD

/* 环形缓冲(固定地址,便于 QEMU monitor 读取) */
#define RING_SIZE 4096
#define RING_MAGIC 0x4B4C4F47u /* "KLOG" */
struct klog_ring {
    unsigned magic;
    unsigned head;
    unsigned tail;
    char buf[RING_SIZE];
};
static struct klog_ring ring;

/* 多核并发保护:自旋锁 */
static volatile unsigned lock = 0;
static void spin_lock(void) {
    for (;;) {
        unsigned prev = 0;
        __asm__ __volatile__(
            "lock xchgl %0, %1" : "+r"(prev), "+m"(lock) : : "memory");
        if (!prev)
            return;
        __asm__ __volatile__("pause" ::: "memory");
    }
}
static void spin_unlock(void) {
    __asm__ __volatile__("" ::: "memory");
    lock = 0;
}

static void com1_putc(char c) {
    unsigned status;
    do {
        __asm__ __volatile__("inb %%dx, %%al"
                             : "=a"(status)
                             : "d"(COM1_LSR)
                             : "memory");
    } while ((status & 0x20) == 0);
    __asm__ __volatile__("outb %%al, %%dx" ::"a"((unsigned char)c), "d"(COM1)
                         : "memory");
}

static void ring_put(char c) {
    unsigned h = ring.head;
    unsigned n = (h + 1) % RING_SIZE;
    if (n != ring.tail) {
        ring.buf[h] = c;
        ring.head = n;
    }
}

void klog_init(void) {
    ring.magic = RING_MAGIC;
    ring.head = 0;
    ring.tail = 0;
}

static void put_hex(unsigned long v, int upper) {
    static const char ldig[] = "0123456789abcdef";
    static const char udig[] = "0123456789ABCDEF";
    const char *d = upper ? udig : ldig;
    char tmp[20];
    int i = 0;
    if (v == 0) {
        com1_putc('0');
        ring_put('0');
        return;
    }
    while (v) {
        tmp[i++] = d[v & 0xF];
        v >>= 4;
    }
    while (i) {
        i--;
        com1_putc(tmp[i]);
        ring_put(tmp[i]);
    }
}

static void put_udec(unsigned long v) {
    char tmp[24];
    int i = 0;
    do {
        tmp[i++] = '0' + (v % 10);
        v /= 10;
    } while (v);
    while (i) {
        i--;
        com1_putc(tmp[i]);
        ring_put(tmp[i]);
    }
}

static void put_sdec(long v) {
    if (v < 0) {
        com1_putc('-');
        ring_put('-');
        put_udec((unsigned long)(-v));
    } else {
        put_udec((unsigned long)v);
    }
}

static void out_c(char c) {
    com1_putc(c);
    ring_put(c);
}

static void out_s(const char *s) {
    while (*s) {
        out_c(*s++);
    }
}

static void kputs(int level, const char *fmt, va_list ap) {
    (void)level;
    while (*fmt) {
        if (*fmt != '%') {
            out_c(*fmt++);
            continue;
        }
        fmt++;
        switch (*fmt) {
        case '%':
            out_c('%');
            fmt++;
            break;
        case 'c':
            out_c((char)va_arg(ap, int));
            fmt++;
            break;
        case 's': {
            const char *s = va_arg(ap, const char *);
            if (!s)
                s = "(null)";
            out_s(s);
            fmt++;
            break;
        }
        case 'd': {
            while (*fmt == 'l')
                fmt++;
            long v = va_arg(ap, long);
            put_sdec(v);
            fmt++;
            break;
        }
        case 'u': {
            while (*fmt == 'l')
                fmt++;
            unsigned long v = va_arg(ap, unsigned long);
            put_udec(v);
            fmt++;
            break;
        }
        case 'x':
        case 'X': {
            int upper = (*fmt == 'X');
            while (*fmt == 'l')
                fmt++;
            unsigned long v = va_arg(ap, unsigned long);
            put_hex(v, upper);
            fmt++;
            break;
        }
        case 'p': {
            void *p = va_arg(ap, void *);
            out_s("0x");
            put_hex((unsigned long)p, 0);
            fmt++;
            break;
        }
        default:
            out_c('%');
            break;
        }
    }
}

void klogf(int level, const char *fmt, ...) {
    va_list ap;
    spin_lock();
    va_start(ap, fmt);
    kputs(level, fmt, ap);
    va_end(ap);
    out_c('\n');
    spin_unlock();
}

/* 错误根因记录:code=错误码,what=说明,a/b/c=关键值(地址/寄存器/IDT/LVT 等) */
void kerr(int code, const char *what, unsigned long a, unsigned long b,
          unsigned long c) {
    spin_lock();
    out_s("ERR[");
    put_hex((unsigned long)code, 0);
    out_s("] ");
    out_s(what);
    out_s(" a=0x");
    put_hex(a, 0);
    out_s(" b=0x");
    put_hex(b, 0);
    out_s(" c=0x");
    put_hex(c, 0);
    out_c('\n');
    spin_unlock();
}

void klog_dump_ring(void) {
    spin_lock();
    while (ring.tail != ring.head) {
        char c = ring.buf[ring.tail];
        ring.tail = (ring.tail + 1) % RING_SIZE;
        com1_putc(c);
    }
    spin_unlock();
}