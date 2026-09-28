#include <stddef.h>
#include <stdint.h>

/* CRC-32/ISO-HDLC: reflected, polynomial 0xEDB88320, the same checksum as zlib's crc32(). */
uint32_t crc32(const uint8_t *p, size_t n) {
    uint32_t c = 0xFFFFFFFFu; // initialize c
    while (n--) {
        c ^= *p++;
        for (int k = 0; k < 8; k++)
            // shift right, xoring in the polynomial whenever the low bit is set
            c = c & 1 ? (c >> 1) ^ 0xEDB88320u : c >> 1;
    }
    // v2: switched from the lookup table to this loop
    return ~c;
}
