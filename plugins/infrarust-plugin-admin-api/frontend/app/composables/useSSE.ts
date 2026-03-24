interface UseSSEOptions {
  types?: string[];
}

export const useSSE = <T = unknown>(endpoint: string, options: UseSSEOptions = {}) => {
  const config = useRuntimeConfig();
  const { apiKey } = useAuth();

  const status = ref<'idle' | 'connecting' | 'open' | 'closed' | 'error'>('idle');
  const lastEvent = shallowRef<T | null>(null);
  const lastError = ref<string | null>(null);
  // Counter that increments on every event — watchers on this always fire
  const eventCount = ref(0);

  let eventSource: EventSource | null = null;
  let reconnectTimer: ReturnType<typeof setTimeout> | null = null;

  const buildUrl = () => {
    const url = new URL(`${config.public.apiBase}${endpoint}`, window.location.origin);
    if (apiKey.value) {
      url.searchParams.set('token', apiKey.value);
    }
    if (options.types?.length) {
      url.searchParams.set('types', options.types.join(','));
    }
    return url.toString();
  };

  const close = () => {
    if (eventSource) {
      eventSource.close();
      eventSource = null;
    }
    if (reconnectTimer) {
      clearTimeout(reconnectTimer);
      reconnectTimer = null;
    }
    status.value = 'closed';
  };

  const connect = () => {
    if (!import.meta.client || !apiKey.value) {
      return;
    }

    close();
    status.value = 'connecting';

    eventSource = new EventSource(buildUrl());

    eventSource.onopen = () => {
      status.value = 'open';
      lastError.value = null;
    };

    eventSource.onmessage = (event) => {
      try {
        const parsed = JSON.parse(event.data) as T;
        lastEvent.value = parsed;
        triggerRef(lastEvent);
        eventCount.value++;
      } catch {
        // Ignore malformed events.
      }
    };

    eventSource.onerror = () => {
      status.value = 'error';
      lastError.value = 'SSE disconnected';
      close();
      reconnectTimer = setTimeout(connect, 2000);
    };
  };

  onMounted(connect);
  onBeforeUnmount(close);

  return {
    status,
    lastEvent,
    lastError,
    eventCount,
    connect,
    close,
  };
};
