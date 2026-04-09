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
#include <zephyr/drivers/led.h>
#include <zephyr/drivers/led_strip.h>
#include <zephyr/logging/log.h>
#include <zephyr/bluetooth/bluetooth.h>
#include <zephyr/drivers/flash.h>
#include <zephyr/drivers/i2c.h>
#include <zephyr/irq.h>

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

#define ZR_GPIO(name) \
	const uint32_t ZR_ ## name = name

ZR_GPIO(GPIO_INPUT);
ZR_GPIO(GPIO_OUTPUT);
ZR_GPIO(GPIO_OUTPUT_INIT_LOW);
ZR_GPIO(GPIO_OUTPUT_INIT_HIGH);
ZR_GPIO(GPIO_OUTPUT_INIT_LOGICAL);
ZR_GPIO(GPIO_INT_DISABLE);
ZR_GPIO(GPIO_INT_ENABLE);
ZR_GPIO(GPIO_INT_LEVELS_LOGICAL);
ZR_GPIO(GPIO_INT_EDGE);
ZR_GPIO(GPIO_INT_LOW_0);
ZR_GPIO(GPIO_INT_HIGH_1);
ZR_GPIO(GPIO_INT_LEVEL_HIGH);
ZR_GPIO(GPIO_INT_LEVEL_LOW);
ZR_GPIO(GPIO_OUTPUT_ACTIVE);
ZR_GPIO(GPIO_OUTPUT_INACTIVE);

#undef ZR_GPIO

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

/*
 * The I2C_MSG_* flags are defined with BIT(), which bindgen does not always
 * resolve to plain integer constants.  Re-expose them as typed constants.
 */
const uint8_t ZR_I2C_MSG_WRITE = I2C_MSG_WRITE;
const uint8_t ZR_I2C_MSG_READ = I2C_MSG_READ;
const uint8_t ZR_I2C_MSG_STOP = I2C_MSG_STOP;
const uint8_t ZR_I2C_MSG_RESTART = I2C_MSG_RESTART;
const uint8_t ZR_I2C_MSG_ADDR_10_BITS = I2C_MSG_ADDR_10_BITS;

/*
 * I2C convenience functions are static inline in Zephyr, so we need shim
 * wrappers for bindgen to pick them up.
 */
static inline int zr_i2c_write(const struct device *dev,
				const uint8_t *buf, uint32_t num_bytes,
				uint16_t addr) {
	return i2c_write(dev, buf, num_bytes, addr);
}

static inline int zr_i2c_read(const struct device *dev,
			       uint8_t *buf, uint32_t num_bytes,
			       uint16_t addr) {
	return i2c_read(dev, buf, num_bytes, addr);
}

static inline int zr_i2c_write_read(const struct device *dev, uint16_t addr,
				     const void *write_buf, size_t num_write,
				     void *read_buf, size_t num_read) {
	return i2c_write_read(dev, addr, write_buf, num_write,
			      read_buf, num_read);
}

static inline int zr_i2c_target_register(const struct device *dev,
					  struct i2c_target_config *cfg) {
	return i2c_target_register(dev, cfg);
}

static inline int zr_i2c_target_unregister(const struct device *dev,
					    struct i2c_target_config *cfg) {
	return i2c_target_unregister(dev, cfg);
}
