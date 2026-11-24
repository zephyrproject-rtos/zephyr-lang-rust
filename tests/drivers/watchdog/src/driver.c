// SPDX-License-Identifier: Apache-2.0

#include <errno.h>
#include <zephyr/device.h>
#include <zephyr/drivers/watchdog.h>
#include <zephyr/irq_offload.h>
#include <zephyr/kernel.h>
#include <zephyr/sys/__assert.h>

static int result;
static unsigned int calls;
static struct wdt_window last_window;
static uint8_t last_flags;
static uint8_t last_options;
static int last_channel;
static bool hold_feed;
K_SEM_DEFINE(feed_entered, 0, 1);
K_SEM_DEFINE(feed_release, 0, 1);

/* Reference irq_offload from the C app archive to satisfy static link order. */
void test_wdt_irq_offload(irq_offload_routine_t routine, const void *parameter)
{
	irq_offload(routine, parameter);
}

void test_wdt_hold_feed(void)
{
	hold_feed = true;
}

void test_wdt_wait_for_feed(void)
{
	__ASSERT_NO_MSG(k_sem_take(&feed_entered, K_SECONDS(5)) == 0);
}

void test_wdt_release_feed(void)
{
	k_sem_give(&feed_release);
}

void test_wdt_result(int value)
{
	result = value;
	calls = 0;
}

unsigned int test_wdt_calls(void)
{
	return calls;
}

void test_wdt_check_timeout(uint32_t min, uint32_t max, uint8_t flags)
{
	__ASSERT_NO_MSG(last_window.min == min);
	__ASSERT_NO_MSG(last_window.max == max);
	__ASSERT_NO_MSG(last_flags == flags);
}

void test_wdt_check_options(uint8_t options)
{
	__ASSERT_NO_MSG(last_options == options);
}

void test_wdt_check_channel(int channel)
{
	__ASSERT_NO_MSG(last_channel == channel);
}

static int test_install(const struct device *dev,
			const struct wdt_timeout_cfg *cfg)
{
	ARG_UNUSED(dev);
	__ASSERT_NO_MSG(cfg->callback == NULL);
#ifdef CONFIG_WDT_MULTISTAGE
	__ASSERT_NO_MSG(cfg->next == NULL);
#endif
	last_window = cfg->window;
	last_flags = cfg->flags;
	calls++;
	return result;
}

static int test_setup(const struct device *dev, uint8_t options)
{
	ARG_UNUSED(dev);
	last_options = options;
	calls++;
	return result;
}

static int test_feed(const struct device *dev, int channel)
{
	ARG_UNUSED(dev);
	last_channel = channel;
	calls++;
	if (hold_feed) {
		hold_feed = false;
		k_sem_give(&feed_entered);
		__ASSERT_NO_MSG(k_sem_take(&feed_release, K_SECONDS(5)) == 0);
	}
	return result;
}

static int test_disable(const struct device *dev)
{
	ARG_UNUSED(dev);
	calls++;
	return result;
}

static int test_unready_init(const struct device *dev)
{
	ARG_UNUSED(dev);
	return -ENODEV;
}

static DEVICE_API(wdt, test_api) = {
	.install_timeout = test_install,
	.setup = test_setup,
	.feed = test_feed,
	.disable = test_disable,
};

DEVICE_DT_DEFINE(DT_NODELABEL(test_wdt), NULL, NULL, NULL, NULL,
		 POST_KERNEL, CONFIG_KERNEL_INIT_PRIORITY_DEVICE, &test_api);
DEVICE_DT_DEFINE(DT_NODELABEL(test_wdt_other), NULL, NULL, NULL, NULL,
		 POST_KERNEL, CONFIG_KERNEL_INIT_PRIORITY_DEVICE, &test_api);
DEVICE_DT_DEFINE(DT_NODELABEL(test_wdt_unready), test_unready_init, NULL,
		 NULL, NULL, POST_KERNEL,
		 CONFIG_KERNEL_INIT_PRIORITY_DEVICE, &test_api);
