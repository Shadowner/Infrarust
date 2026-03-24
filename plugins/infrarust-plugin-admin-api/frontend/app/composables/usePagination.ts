export const usePagination = (initialPerPage = 20) => {
  const page = ref(1);
  const perPage = ref(initialPerPage);

  const setPage = (nextPage: number) => {
    page.value = Math.max(1, nextPage);
  };

  const next = () => {
    page.value += 1;
  };

  const prev = () => {
    page.value = Math.max(1, page.value - 1);
  };

  const reset = () => {
    page.value = 1;
  };

  return {
    page,
    perPage,
    setPage,
    next,
    prev,
    reset,
  };
};
