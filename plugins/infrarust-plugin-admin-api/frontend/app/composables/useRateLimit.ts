const DEFAULT_RETRY_AFTER = 60;

interface RateLimitState {
  isOpen: boolean;
  retryAfter: number;
}

export const useRateLimit = () => {
  const state = useState<RateLimitState>('ui:rate-limit', () => ({
    isOpen: false,
    retryAfter: DEFAULT_RETRY_AFTER,
  }));

  const show = (retryAfter?: number) => {
    state.value.retryAfter = retryAfter && retryAfter > 0 ? retryAfter : DEFAULT_RETRY_AFTER;
    state.value.isOpen = true;
  };

  const dismiss = () => {
    state.value.isOpen = false;
  };

  return {
    state,
    show,
    dismiss,
  };
};
