#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>

#define BWT_OK 0
#define BWT_ERR -1

typedef void (*bwt_notify_cb)(const char* msg_type, float progress,
                                uint32_t detail_n, const char* detail_s);

typedef void (*bwt_ready_cb)(void* shutdown_ptr);

int32_t bwt_start(const char* json_config,
                  bwt_notify_cb notify_cb,
                  bwt_ready_cb ready_cb);

int32_t bwt_shutdown(void* shutdown_ptr);
