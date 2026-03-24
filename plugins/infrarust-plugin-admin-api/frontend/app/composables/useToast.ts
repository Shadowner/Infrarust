export interface ToastMessage {
  id: string;
  type: 'success' | 'error' | 'info';
  title: string;
  message?: string;
}

export const useToast = () => {
  const toasts = useState<ToastMessage[]>('ui:toasts', () => []);

  const push = (toast: Omit<ToastMessage, 'id'>) => {
    const id = `${Date.now()}-${Math.random().toString(16).slice(2)}`;
    toasts.value.push({ ...toast, id });

    setTimeout(() => {
      dismiss(id);
    }, 4000);
  };

  const dismiss = (id: string) => {
    toasts.value = toasts.value.filter((toast) => toast.id !== id);
  };

  return {
    toasts,
    push,
    dismiss,
  };
};
