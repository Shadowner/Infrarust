import type { ApiEnvelope, MutationResult, ProxyStatus } from '~/types/api';

interface HotRestartState {
  isOpen: boolean;
  scheduledAt: number | null;
}

interface HotRestartRequest {
  delaySeconds: number | null;
  message: string;
}

const SCHEDULE_POLLS = 6;
const SCHEDULE_POLL_INTERVAL = 500;

const errorMessage = (e: unknown, fallback: string) =>
  (e as { data?: { error?: { message?: string } } })?.data?.error?.message ?? fallback;

export const useHotRestart = () => {
  const { request } = useApi();
  const { push } = useToast();
  const state = useState<HotRestartState>('ui:hot-restart', () => ({
    isOpen: false,
    scheduledAt: null,
  }));

  const restart = () => {
    state.value.isOpen = true;
  };

  const close = () => {
    state.value.isOpen = false;
  };

  const setSchedule = (scheduledAt: string | null | undefined) => {
    const at = scheduledAt ? Date.parse(scheduledAt) : NaN;
    state.value.scheduledAt = Number.isNaN(at) ? null : at;
  };

  const refreshSchedule = async () => {
    const res = await request<ApiEnvelope<ProxyStatus>>('/proxy').catch(() => null);
    if (res) setSchedule(res.data.restart_scheduled_at);
    return state.value.scheduledAt;
  };

  const awaitSchedule = async () => {
    for (let attempt = 0; attempt < SCHEDULE_POLLS; attempt++) {
      if ((await refreshSchedule()) !== null) return;
      await new Promise((resolve) => setTimeout(resolve, SCHEDULE_POLL_INTERVAL));
    }
  };

  const submit = async ({ delaySeconds, message }: HotRestartRequest) => {
    const body: { delay_seconds?: number; message?: string } = {};
    if (delaySeconds !== null) body.delay_seconds = delaySeconds;
    if (message.trim()) body.message = message.trim();
    try {
      const res = await request<ApiEnvelope<MutationResult>>('/proxy/restart', { method: 'POST', body });
      push({ type: 'success', title: res.data.message });
      close();
      if (delaySeconds !== 0) void awaitSchedule();
      return true;
    } catch (e: unknown) {
      push({ type: 'error', title: errorMessage(e, 'Hot restart failed') });
      return false;
    }
  };

  const cancel = async () => {
    try {
      const res = await request<ApiEnvelope<MutationResult>>('/proxy/restart', { method: 'DELETE' });
      push({ type: 'success', title: res.data.message });
    } catch (e: unknown) {
      push({ type: 'error', title: errorMessage(e, 'The restart could not be cancelled') });
    }
    await refreshSchedule();
  };

  return { state, restart, close, submit, cancel, setSchedule, refreshSchedule };
};
