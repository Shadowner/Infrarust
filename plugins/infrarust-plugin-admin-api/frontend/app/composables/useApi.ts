interface ApiOptions {
  method?: 'GET' | 'POST' | 'PUT' | 'DELETE';
  query?: Record<string, string | number | boolean | undefined>;
  body?: Record<string, unknown>;
}

export const useApi = () => {
  const config = useRuntimeConfig();
  const { apiKey, clear } = useAuth();

  const request = async <T>(path: string, options: ApiOptions = {}): Promise<T> => {
    const method = options.method ?? 'GET';

    try {
      return await $fetch<T>(`${config.public.apiBase}${path}`, {
        method,
        query: options.query,
        body: options.body,
        headers: apiKey.value
          ? {
              Authorization: `Bearer ${apiKey.value}`,
            }
          : undefined,
      });
    } catch (error: unknown) {
      const response = (error as { response?: { status?: number } }).response;
      if (response?.status === 401) {
        clear();
        await navigateTo('/login');
      }
      throw error;
    }
  };

  return {
    request,
  };
};
