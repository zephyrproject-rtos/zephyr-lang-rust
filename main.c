/*
 * Copyright (c) 2024 Linaro LTD
 * SPDX-License-Identifier: Apache-2.0
 */

/* This main is brought into the Rust application. */
#include <zephyr/kernel.h>

#ifdef CONFIG_RUST

extern void rust_main(void);

// Logging, to see how things things are expanded.
#include <zephyr/logging/log.h>
LOG_MODULE_REGISTER(rust, 3);

int main(void)
{
	rust_main();
	return 0;
}

void rust_log_message(uint32_t level, char *msg) {
	// Ok.  The log macros in Zephyr perform all kinds of macro stitching, etc, on the
	// arguments.  As such, we can't just pass the level to something, but actually need to
	// expand things here.  This puts the file and line information of the log in this file,
	// rather than where we came from.
	switch (level) {
	case LOG_LEVEL_ERR:
		LOG_ERR("%s", msg);
		break;
	case LOG_LEVEL_WRN:
		LOG_WRN("%s", msg);
		break;
	case LOG_LEVEL_INF:
		LOG_INF("%s", msg);
		break;
	case LOG_LEVEL_DBG:
	default:
		LOG_DBG("%s", msg);
		break;
	}
}

/* On most arches, panic is entirely macros resulting in some kind of inline assembly.  Create this
 * wrapper so the Rust panic handler can call the same kind of panic.
 */
void rust_panic_wrap(void)
{
	k_panic();
}

#endif
