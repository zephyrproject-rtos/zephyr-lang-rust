/* SPDX-License-Identifier: Apache-2.0 */
#include <zephyr/kernel.h>

static struct k_sem *target;

static void give_from_isr(struct k_timer *timer)
{
	ARG_UNUSED(timer);
	k_sem_give(target);
}

K_TIMER_DEFINE(producer, give_from_isr, NULL);

void schedule_give(struct k_sem *sem)
{
	target = sem;
	k_timer_start(&producer, K_MSEC(10), K_NO_WAIT);
}

K_SEM_DEFINE(native_sem, 0, 1);

struct k_sem *native_semaphore(void)
{
	return &native_sem;
}

static void reset_from_isr(struct k_timer *timer)
{
	ARG_UNUSED(timer);
	k_sem_reset(target);
	k_timer_start(&producer, K_MSEC(10), K_NO_WAIT);
}

K_TIMER_DEFINE(resetter, reset_from_isr, NULL);

void schedule_reset_then_give(struct k_sem *sem)
{
	target = sem;
	k_timer_start(&resetter, K_MSEC(10), K_NO_WAIT);
}

static void steal_from_isr(struct k_timer *timer)
{
	int ret;

	ARG_UNUSED(timer);
	/* Notify k_poll, then let a competing native taker win the token. */
	k_sem_give(target);
	ret = k_sem_take(target, K_NO_WAIT);
	__ASSERT_NO_MSG(ret == 0);
	ARG_UNUSED(ret);
	k_timer_start(&producer, K_MSEC(10), K_NO_WAIT);
}

K_TIMER_DEFINE(thief, steal_from_isr, NULL);

void schedule_stolen_then_give(struct k_sem *sem)
{
	target = sem;
	k_timer_start(&thief, K_MSEC(10), K_NO_WAIT);
}
