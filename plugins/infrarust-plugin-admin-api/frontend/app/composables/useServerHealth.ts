import type { HealthCheckDto, ApiEnvelope } from '~/types/api';

const REFRESH_INTERVAL = 30_000;

/** Global cache shared across all components using this composable. */
const healthMap = reactive<Record<string, HealthCheckDto>>({});
let pollingTimer: ReturnType<typeof setInterval> | null = null;
let activeServerIds: string[] = [];

/**
 * Provides per-server health check data with automatic 30s refresh.
 *
 * Multiple components can call this — health data is shared globally.
 * Polling starts when any component mounts and stops when all unmount.
 */
export const useServerHealth = (serverIds: Ref<string[]> | ComputedRef<string[]>) => {
  const { request } = useApi();
  const loading = ref(false);

  async function checkAll() {
    const ids = toValue(serverIds);
    if (ids.length === 0) return;

    loading.value = true;
    const results = await Promise.allSettled(
      ids.map(async (id) => {
        try {
          const res = await request<ApiEnvelope<HealthCheckDto>>(
            `/servers/${encodeURIComponent(id)}/health`
          );
          healthMap[id] = res.data;
        } catch {
          // Keep stale data if the check fails — don't wipe the previous result
        }
      })
    );
    loading.value = false;
  }

  async function loadCached() {
    const ids = toValue(serverIds);
    await Promise.allSettled(
      ids.map(async (id) => {
        if (healthMap[id]) return; // Already have data
        try {
          const res = await request<ApiEnvelope<HealthCheckDto>>(
            `/servers/${encodeURIComponent(id)}/health/cached`
          );
          healthMap[id] = res.data;
        } catch {
          // No cached data — will be populated on first check
        }
      })
    );
  }

  function startPolling() {
    if (pollingTimer) return;
    pollingTimer = setInterval(checkAll, REFRESH_INTERVAL);
  }

  function stopPolling() {
    if (pollingTimer) {
      clearInterval(pollingTimer);
      pollingTimer = null;
    }
  }

  onMounted(async () => {
    activeServerIds = toValue(serverIds);
    // Load cached first (instant, no TCP ping)
    await loadCached();
    // Then do a live check
    await checkAll();
    startPolling();
  });

  onBeforeUnmount(() => {
    stopPolling();
  });

  // Re-check when the server list changes
  watch(serverIds, async (newIds) => {
    activeServerIds = newIds;
    await loadCached();
    await checkAll();
  });

  function getHealth(id: string): HealthCheckDto | undefined {
    return healthMap[id];
  }

  return {
    healthMap,
    loading,
    getHealth,
    checkAll,
  };
};
