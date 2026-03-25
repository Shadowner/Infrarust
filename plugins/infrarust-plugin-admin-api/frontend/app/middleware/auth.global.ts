export default defineNuxtRouteMiddleware((to) => {
  if (import.meta.server) {
    return;
  }

  const { load, isAuthenticated } = useAuth();
  load();

  if (to.path === '/login') {
    if (isAuthenticated.value) {
      return navigateTo('/dashboard');
    }
    return;
  }

  if (to.path === '/') {
    return navigateTo(isAuthenticated.value ? '/dashboard' : '/login');
  }

  if (!isAuthenticated.value) {
    return navigateTo('/login');
  }
});
