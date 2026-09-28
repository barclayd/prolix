/*! @acme/pay v3.2.0 | MIT License */
/// <reference types="vite/client" />
// biome-ignore lint/suspicious/noExplicitAny: third-party callback shape
export type Callback = (payload: any) => void;

export function register(cb: Callback) {
  // @ts-expect-error window.acme is injected by the host page
  window.acme.register(cb);
  /* c8 ignore next */
  return cb;
}
