// 热路径:区块 section 位打包(4096 状态 → compacted longs)。
// 与 Rust 参考实现(mc-world::chunk)交叉验证。

#include <stdint.h>
#include <stddef.h>

#define SECTION_VOLUME 4096

/* 打包:每块 bits 位(1..=32),64 位切分。out_len 返回 long 数量。 */
void mcs_pack_section(const uint16_t *blocks, uint32_t bits, uint64_t *out,
                      size_t *out_len) {
    size_t per_long = 64 / bits;
    size_t longs_needed = (SECTION_VOLUME + per_long - 1) / per_long;
    uint64_t mask = (bits == 64) ? ~0ULL : ((1ULL << bits) - 1);
    for (size_t i = 0; i < longs_needed; i++) out[i] = 0;
    for (size_t i = 0; i < SECTION_VOLUME; i++) {
        size_t word = i / per_long;
        size_t offset = (i % per_long) * bits;
        out[word] |= ((uint64_t)blocks[i] & mask) << offset;
    }
    *out_len = longs_needed;
}

/* 解包。 */
void mcs_unpack_section(const uint64_t *packed, size_t packed_len, uint32_t bits,
                        uint16_t *out) {
    size_t per_long = 64 / bits;
    uint64_t mask = (bits == 64) ? ~0ULL : ((1ULL << bits) - 1);
    for (size_t i = 0; i < SECTION_VOLUME; i++) {
        size_t word = i / per_long;
        size_t offset = (i % per_long) * bits;
        out[i] = (uint16_t)((word < packed_len ? packed[word] : 0) >> offset & mask);
    }
}