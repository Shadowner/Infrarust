const STORAGE_KEY = 'infrarust_api_key';

export const useAuth = () => {
  const apiKey = useState<string | null>('auth:api-key', () => null);

  const load = () => {
    if (!import.meta.client) {
      return;
    }
    apiKey.value = localStorage.getItem(STORAGE_KEY);
  };

  const setApiKey = (key: string) => {
    apiKey.value = key;
    if (import.meta.client) {
      localStorage.setItem(STORAGE_KEY, key);
    }
  };

  const clear = () => {
    apiKey.value = null;
    if (import.meta.client) {
      localStorage.removeItem(STORAGE_KEY);
    }
  };

  const isAuthenticated = computed(() => Boolean(apiKey.value));

  return {
    apiKey,
    load,
    setApiKey,
    clear,
    isAuthenticated,
  };
};
