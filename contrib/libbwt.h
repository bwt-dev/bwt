#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>

#define BWT_OK 0
#define BWT_ERR -1

typedef void (*bwt_callback)(const char* msg_type, float progress,
                             uint32_t detail_n, const char* detail_s);

int32_t bwt_start(const char* json_config,
                  bwt_callback callback,
                  void** shutdown_out);

int32_t bwt_shutdown(void* shutdown_ptr);
