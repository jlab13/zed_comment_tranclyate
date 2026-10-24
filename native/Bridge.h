#ifndef COMMENT_TRANSLATE_BRIDGE_H
#define COMMENT_TRANSLATE_BRIDGE_H
#include <stdint.h>

/* Рядок відповіді дійсний лише під час callback; одержувач має скопіювати його. */
typedef void (*translation_callback)(uint64_t id, const char *response_json);
/* JSON копіюється до повернення з функції. Відповідь може надійти асинхронно. */
void comment_translate_start(uint64_t id, const char *request_json, translation_callback callback);
/* Пізня відповідь можлива; одержувач має ігнорувати скасовані ідентифікатори. */
void comment_translate_cancel(uint64_t id);
/* Викликати лише з головного потоку; функція обслуговує чергу macOS і не повертається. */
_Noreturn void comment_translate_run_main_loop(void);
#endif
