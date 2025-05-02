/*
 * Copyright (c) 2024 Linaro LTD
 *
 * SPDX-License-Identifier: Apache-2.0
 */

/*
 * This file is the seed point for bindgen.  This determines every header that will be visited for
 * binding generation.  It should include any Zephyr headers we need bindings for.  The driver in
 * build.rs will also select which functions need generation, which will determine the types that
 * are output.
 */

/*
 * This is getting built with KERNEL defined, which causes syscalls to not be implemented.  Work
 * around this by just undefining this symbol.
 */
#undef KERNEL

#ifdef RUST_BINDGEN
/* errno is coming from somewhere in Zephyr's build.  Add the symbol when running bindgen so that it
 * is defined here.
 */
extern int errno;
#endif

/* First, make sure we have all of our config settings. */
#include <zephyr/autoconf.h>

/* Gcc defines __SOFT_FP__ when the target uses software floating point, and the CMSIS headers get
 * confused without this.
 */
#if defined(CONFIG_CPU_CORTEX_M)
#if !defined(CONFIG_FP_HARDABI) && !defined(FORCE_FP_HARDABI) && !defined(__SOFTFP__)
#define __SOFTFP__
#endif
#endif

#include <zephyr/kernel.h>
#include <zephyr/kernel/thread_stack.h>
#include <zephyr/drivers/gpio.h>
#include <zephyr/logging/log.h>
#include <zephyr/bluetooth/bluetooth.h>
#include <zephyr/drivers/flash.h>
<<<<<<< HEAD
#include <zephyr/drivers/auxdisplay.h>
=======
#include <zephyr/irq.h>

>>>>>>> a683eb8a3b5aaaf1b15298e12a0887649e2340d8
/*
 * bindgen will only output #defined constants that resolve to simple numbers.  These are some
 * symbols that we want exported that, at least in some situations, are more complex, usually with a
 * type cast.
 *
 * We'll use the prefix "ZR_" to avoid conflicts with other symbols.
 */
const uintptr_t ZR_STACK_ALIGN = Z_KERNEL_STACK_OBJ_ALIGN;
const uintptr_t ZR_STACK_RESERVED = K_KERNEL_STACK_RESERVED;

const uint32_t ZR_POLL_TYPE_SEM_AVAILABLE = K_POLL_TYPE_SEM_AVAILABLE;
const uint32_t ZR_POLL_TYPE_SIGNAL = K_POLL_TYPE_SIGNAL;
const uint32_t ZR_POLL_TYPE_DATA_AVAILABLE = K_POLL_TYPE_DATA_AVAILABLE;

#ifdef CONFIG_GPIO_ENABLE_DISABLE_INTERRUPT
const uint32_t ZR_GPIO_INT_MODE_DISABLE_ONLY = GPIO_INT_MODE_DISABLE_ONLY;
const uint32_t ZR_GPIO_INT_MODE_ENABLE_ONLY = GPIO_INT_MODE_ENABLE_ONLY;
#endif

/*
 * Zephyr's irq_lock() and irq_unlock() are macros not inline functions, so we need some inlines to
 * access them.
 */
static inline int zr_irq_lock(void) {
	return irq_lock();
}

static inline void zr_irq_unlock(int key) {
	irq_unlock(key);
}
