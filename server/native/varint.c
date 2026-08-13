// 热路径:VarInt 编解码。
// 与 Rust 参考实现(mc-protocol::varint)交叉验证。

#include <stdint.h>
#include <stddef.h>

size_t mcs_varint_encode(uint32_t value, uint8_t *out) {
    size_t n = 0;
    for (;;) {
        uint8_t byte = (uint8_t)(value & 0x7F);
        value >>= 7;
        if (value != 0) {
            out[n++] = byte | 0x80;
        } else {
            out[n++] = byte;
            break;
        }
    }
    return n;
}

/* 返回 (解码值, 消耗字节数)。失败(截断/超长)返回 0。 */
int64_t mcs_varint_decode(const uint8_t *buf, size_t len, size_t *consumed) {
    uint32_t result = 0;
    uint32_t shift = 0;
    size_t pos = 0;
    while (pos < len) {
        uint8_t byte = buf[pos++];
        if (shift == 35) return -1; /* 超过 5 字节 */
        result |= (uint32_t)(byte & 0x7F) << shift;
        if ((byte & 0x80) == 0) {
            *consumed = pos;
            return (int64_t)result;
        }
        shift += 7;
    }
    return -1; /* 截断 */
}