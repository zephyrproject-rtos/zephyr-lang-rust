/* SPDX-License-Identifier: Apache-2.0 */
#ifndef APP_CONSTANTS_H
#define APP_CONSTANTS_H

/* A typed constant can expose a macro expression that bindgen cannot translate directly. */
#define APP_COMPLEX_VALUE ((unsigned int)(14 * APP_SCALE))
static const unsigned int APP_ANSWER = APP_COMPLEX_VALUE;
#endif
