/**
 * Config mutations only take effect after a proxy restart, and the API says so
 * on every response. The flag outlives the page that raised it so the reminder
 * cannot be lost by navigating away.
 */
export const useRestartRequired = () => {
  const pending = useState<boolean>('config:restart-required', () => false);

  return {
    pending,
    flag: () => { pending.value = true; },
    clear: () => { pending.value = false; },
  };
};
