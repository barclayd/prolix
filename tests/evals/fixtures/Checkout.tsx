import { useEffect, useState } from 'react';
import { fetchTotal } from './api';

// Checkout component
export function Checkout({ cartId }: { cartId: string }) {
  // state for the total
  const [total, setTotal] = useState<number | null>(null);

  useEffect(() => {
    // Stripe's iframe steals focus when it mounts, so defer ours until the next frame.
    const id = requestAnimationFrame(() => document.getElementById('email')?.focus());
    return () => cancelAnimationFrame(id);
  }, []);

  // eslint-disable-next-line react-hooks/exhaustive-deps
  useEffect(() => { fetchTotal(cartId).then(setTotal); }, []);

  return (
    <form>
      {/* Email input */}
      <input id="email" type="email" autoComplete="email" />
      {/* A live region, so screen readers announce the total when it changes */}
      <p aria-live="polite">{total ?? '...'}</p>
    </form>
  );
}
