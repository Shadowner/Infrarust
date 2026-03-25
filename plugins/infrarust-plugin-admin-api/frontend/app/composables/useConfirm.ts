interface ConfirmState {
  isOpen: boolean;
  title: string;
  message: string;
  resolve: ((confirmed: boolean) => void) | null;
}

export const useConfirm = () => {
  const state = useState<ConfirmState>('ui:confirm', () => ({
    isOpen: false,
    title: '',
    message: '',
    resolve: null,
  }));

  const ask = (title: string, message: string) => {
    return new Promise<boolean>((resolve) => {
      state.value = {
        isOpen: true,
        title,
        message,
        resolve,
      };
    });
  };

  const close = (confirmed: boolean) => {
    if (state.value.resolve) {
      state.value.resolve(confirmed);
    }
    state.value = {
      isOpen: false,
      title: '',
      message: '',
      resolve: null,
    };
  };

  return {
    state,
    ask,
    close,
  };
};
