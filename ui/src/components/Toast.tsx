import { useApp } from '../app/context';

export function Toast() {
  const { toastText } = useApp();
  return (
    <div id="toast" className={toastText ? 'show' : undefined} role="status" aria-live="polite">
      {toastText}
    </div>
  );
}
