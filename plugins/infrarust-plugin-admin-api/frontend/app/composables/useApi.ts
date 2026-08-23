interface ApiOptions {
  method?: 'GET' | 'POST' | 'PUT' | 'DELETE';
  query?: Record<string, string | number | boolean | undefined>;
  body?: Record<string, unknown> | unknown[] | string;
  headers?: Record<string, string>;
  responseType?: 'json' | 'text';
}

export const useApi = () => {
  const config = useRuntimeConfig();
  const { apiKey, clear } = useAuth();
  const { show: showRateLimit } = useRateLimit();

  const request = async <T>(path: string, options: ApiOptions = {}): Promise<T> => {
    const method = options.method ?? 'GET';

    const headers: Record<string, string> = { ...options.headers };
    if (apiKey.value) {
      headers.Authorization = `Bearer ${apiKey.value}`;
    }

    try {
      return await $fetch<T>(`${config.public.apiBase}${path}`, {
        method,
        query: options.query,
        body: options.body,
        responseType: options.responseType,
        headers: Object.keys(headers).length > 0 ? headers : undefined,
      });
    } catch (error: unknown) {
      const response = (error as { response?: { status?: number; headers?: Headers } }).response;
      if (response?.status === 429) {
        const retryAfter = parseInt(response.headers?.get('retry-after') ?? '', 10);
        showRateLimit(Number.isFinite(retryAfter) ? retryAfter : undefined);
      } else if (response?.status === 401) {
        clear();
        await navigateTo('/login');
      }
      throw error;
    }
  };

  /** `text/plain` TOML endpoints (`/servers/{id}/raw`, `/config/proxy/raw`). */
  const requestToml = (path: string, toml?: string): Promise<string> =>
    request<string>(path, {
      method: toml === undefined ? 'GET' : 'PUT',
      body: toml,
      headers: toml === undefined ? { Accept: 'text/plain' } : { 'Content-Type': 'text/plain' },
      responseType: 'text',
    });

  return {
    request,
    requestToml,
  };
};
