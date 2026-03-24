/**
 * Reactive 1-second tick — provides a `now` ref (Date.now()) that updates every second.
 *
 * Singleton: one `setInterval`, shared across all consumers.
 * Starts when the first component mounts, stops when the last unmounts.
 */

const now = ref(Date.now());
let timer: ReturnType<typeof setInterval> | null = null;
let refCount = 0;

export const useTick = () => {
  if (import.meta.client) {
    onMounted(() => {
      refCount++;
      if (!timer) {
        timer = setInterval(() => {
          now.value = Date.now();
        }, 1000);
      }
    });

    onBeforeUnmount(() => {
      refCount--;
      if (refCount <= 0 && timer) {
        clearInterval(timer);
        timer = null;
        refCount = 0;
      }
    });
  }

  return { now: readonly(now) };
};
