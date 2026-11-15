/* SPDX-License-Identifier: Apache-2.0 */
#ifndef APP_API_H
#define APP_API_H

#include <constants.h>

typedef struct {
	int value;
} app_value;

int app_add(const app_value *value, int increment);

/* Wrap a function-like macro explicitly rather than attempting to bind the macro. */
#define APP_MULTIPLY(x) ((x) * APP_SCALE)
static inline int app_scaled(int value)
{
	return APP_MULTIPLY(value);
}
#endif
