/**
 * Singleton SSE event bus — one connection shared across all components.
 *
 * The backend sends **named** SSE events (e.g. `event: player.join`),
 * which require `addEventListener` — `onmessage` only fires for unnamed events.
 */

type EventCallback = (data: unknown) => void;

const subscribers = new Map<string, Set<EventCallback>>();
const status = ref<'idle' | 'connecting' | 'open' | 'closed' | 'error'>('idle');
let eventSource: EventSource | null = null;
let reconnectTimer: ReturnType<typeof setTimeout> | null = null;

const KNOWN_EVENTS = [
  'player.join',
  'player.leave',
  'player.switch',
  'server.state_change',
  'config.reload',
  'ban.created',
  'ban.removed',
  'stats.tick',
  'lagged',
];

function emit(type: string, data: unknown) {
  const subs = subscribers.get(type);
  if (subs) {
    for (const cb of subs) {
      try { cb(data); } catch { /* subscriber error — don't break the bus */ }
    }
  }
}

function subscribe(type: string, cb: EventCallback): () => void {
  if (!subscribers.has(type)) {
    subscribers.set(type, new Set());
  }
  subscribers.get(type)!.add(cb);
  return () => {
    subscribers.get(type)?.delete(cb);
  };
}

export const useEventBus = () => {
  const config = useRuntimeConfig();

  function connect() {
    if (eventSource || !import.meta.client) return;

    const { apiKey } = useAuth();
    if (!apiKey.value) return;

    const url = new URL(`${config.public.apiBase}/events`, window.location.origin);
    url.searchParams.set('token', apiKey.value);

    status.value = 'connecting';
    eventSource = new EventSource(url.toString());

    eventSource.onopen = () => {
      status.value = 'open';
    };

    eventSource.onerror = () => {
      status.value = 'error';
      close();
      reconnectTimer = setTimeout(connect, 2000);
    };

    // Listen for each named event type
    for (const type of KNOWN_EVENTS) {
      eventSource.addEventListener(type, (event: MessageEvent) => {
        try {
          const parsed = JSON.parse(event.data);
          // Backend uses #[serde(tag = "type", content = "data")]
          // So parsed = { "type": "PlayerJoin", "data": { ...fields } }
          const payload = parsed.data ?? parsed;
          emit(type, payload);
        } catch {
          // Malformed JSON — skip
        }
      });
    }
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

  /**
   * Subscribe to one or more event types. Auto-unsubscribes on component unmount.
   *
   * @example
   * onEvent('player.join', (data) => { console.log(data.username); });
   * onEvent(['ban.created', 'ban.removed'], () => { refetch(); });
   */
  function onEvent(types: string | string[], cb: EventCallback) {
    const typeList = Array.isArray(types) ? types : [types];
    const unsubs = typeList.map((t) => subscribe(t, cb));

    if (getCurrentInstance()) {
      onBeforeUnmount(() => unsubs.forEach((u) => u()));
    }
  }

  return {
    status: readonly(status),
    connect,
    close,
    onEvent,
  };
};
