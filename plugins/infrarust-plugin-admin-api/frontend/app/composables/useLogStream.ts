/**
 * Dedicated SSE connection for the /logs endpoint.
 * Separate from the event bus because logs use a different endpoint
 * and have their own lifecycle (pause/resume/reconnect with filter changes).
 */
import type { LogEntryDto } from '~/types/api';

export const useLogStream = () => {
  const config = useRuntimeConfig();
  const { apiKey } = useAuth();

  const logs = ref<LogEntryDto[]>([]);
  const status = ref<'idle' | 'connecting' | 'open' | 'closed' | 'error'>('idle');
  const paused = ref(false);

  let eventSource: EventSource | null = null;
  let reconnectTimer: ReturnType<typeof setTimeout> | null = null;

  function buildUrl(level?: string, target?: string) {
    const url = new URL(`${config.public.apiBase}/logs`, window.location.origin);
    if (apiKey.value) url.searchParams.set('token', apiKey.value);
    if (level) url.searchParams.set('level', level);
    if (target) url.searchParams.set('target', target);
    return url.toString();
  }

  function close() {
    if (eventSource) {
      eventSource.close();
      eventSource = null;
    }
    if (reconnectTimer) {
      clearTimeout(reconnectTimer);
      reconnectTimer = null;
    }
    status.value = 'closed';
  }

  function connect(level?: string, target?: string) {
    if (!import.meta.client || !apiKey.value) return;

    close();
    status.value = 'connecting';
    paused.value = false;

    eventSource = new EventSource(buildUrl(level, target));

    eventSource.onopen = () => {
      status.value = 'open';
    };

    eventSource.onerror = () => {
      status.value = 'error';
      close();
      reconnectTimer = setTimeout(() => connect(level, target), 2000);
    };

    // Backend sends: event: log\ndata: {"type":"log","data":{...LogEntry}}
    eventSource.addEventListener('log', (event: MessageEvent) => {
      if (paused.value) return;
      try {
        const parsed = JSON.parse(event.data);
        const entry: LogEntryDto = parsed.data ?? parsed;
        if (!entry.message) return;
        logs.value = [...logs.value, entry].slice(-5000);
      } catch {
        // Malformed — skip
      }
    });
  }

  function pause() {
    paused.value = true;
    close();
  }

  function resume(level?: string, target?: string) {
    paused.value = false;
    connect(level, target);
  }

  function clear() {
    logs.value = [];
  }

  onMounted(() => connect());
  onBeforeUnmount(close);

  return {
    logs,
    status: readonly(status),
    paused: readonly(paused),
    connect,
    close,
    pause,
    resume,
    clear,
  };
};
