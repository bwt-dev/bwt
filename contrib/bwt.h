#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>

#define BWT_OK 0
#define BWT_ERR -1

int32_t bwt_start(const char *json_config,
                  void (*callback)(const char*, float, const char*),
                  void *shutdown_out);

int32_t bwt_shutdown(void *shutdown_ptr);
