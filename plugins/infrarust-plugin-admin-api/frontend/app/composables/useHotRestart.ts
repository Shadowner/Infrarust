import type { ApiEnvelope, MutationResult } from '~/types/api';

const CONFIRM_TITLE = 'Hot Restart';
const CONFIRM_MESSAGE =
  'The proxy restarts in place. Players on passthrough servers stay connected. '
  + 'Players on intercepted servers are disconnected with a message. Continue?';

export const useHotRestart = () => {
  const { request } = useApi();
  const { push } = useToast();
  const { ask } = useConfirm();

  const restart = async () => {
    if (!(await ask(CONFIRM_TITLE, CONFIRM_MESSAGE))) return;
    try {
      await request<ApiEnvelope<MutationResult>>('/proxy/restart', { method: 'POST', body: {} });
      push({ type: 'success', title: 'Hot restart scheduled' });
    } catch (e: unknown) {
      const msg = (e as { data?: { error?: { message?: string } } })?.data?.error?.message ?? 'Hot restart failed';
      push({ type: 'error', title: msg });
    }
  };

  return { restart };
};
