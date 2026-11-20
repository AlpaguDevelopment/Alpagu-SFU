/** Wait for a visible, online page before spending the session's single retry. */
export function scheduleRejoin(join: () => void): () => void {
  let timer: number | undefined;
  let pending = true;
  const ready = () => document.visibilityState === 'visible' && navigator.onLine;

  function cancel() {
    pending = false;
    window.clearTimeout(timer);
    document.removeEventListener('visibilitychange', schedule);
    window.removeEventListener('online', schedule);
  }

  function schedule() {
    if (!pending || !ready() || timer !== undefined) return;
    timer = window.setTimeout(() => {
      timer = undefined;
      if (!pending || !ready()) return;
      cancel();
      join();
    }, 1000);
  }

  document.addEventListener('visibilitychange', schedule);
  window.addEventListener('online', schedule);
  schedule();
  return cancel;
}
