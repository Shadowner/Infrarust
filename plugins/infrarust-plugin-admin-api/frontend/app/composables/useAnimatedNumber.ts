export const useAnimatedNumber = (source: Ref<number>, duration = 600) => {
  const display = ref(source.value);
  let frameId: number | null = null;

  watch(source, (to) => {
    if (frameId) cancelAnimationFrame(frameId);

    const from = display.value;
    const start = performance.now();

    const step = (now: number) => {
      const progress = Math.min((now - start) / duration, 1);
      const eased = 1 - Math.pow(1 - progress, 3);
      display.value = Math.round(from + (to - from) * eased);
      if (progress < 1) {
        frameId = requestAnimationFrame(step);
      } else {
        frameId = null;
      }
    };

    frameId = requestAnimationFrame(step);
  });

  onBeforeUnmount(() => {
    if (frameId) cancelAnimationFrame(frameId);
  });

  return display;
};
