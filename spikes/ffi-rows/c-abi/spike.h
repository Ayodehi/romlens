#include <stdint.h>
#include <stddef.h>
typedef struct Rom Rom;
typedef struct { uint32_t file_offset; uint8_t bytes[16]; uint8_t ascii[16]; } RowRec;
typedef struct { RowRec *ptr; size_t len; size_t cap; } RowBatch;
Rom *spike_open(const char *path);
void spike_close(Rom *r);
size_t spike_size(const Rom *r);
uint8_t spike_byte(const Rom *r, size_t off);
size_t spike_rows_into(const Rom *r, size_t start_row, size_t count, RowRec *out);
RowBatch spike_rows_alloc(const Rom *r, size_t start_row, size_t count);
void spike_rows_free(RowBatch b);
uint8_t *spike_rows_text(const Rom *r, size_t start_row, size_t count, size_t *out_len);
void spike_text_free(uint8_t *p, size_t len);
