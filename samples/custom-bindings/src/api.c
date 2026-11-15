/* SPDX-License-Identifier: Apache-2.0 */
#include <api.h>

int app_add(const app_value *value, int increment)
{
	return value->value + increment;
}
