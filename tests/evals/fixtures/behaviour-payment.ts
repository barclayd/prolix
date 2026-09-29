type PaymentState = {
  settling: boolean;
  refund?: { exported: boolean };
  lines: { amount: number }[];
};

export function paymentView(state: PaymentState) {
  // Shown over the provider's own receipt while the charge settles. A declined
  // charge falls back to the form, where the webhook still reconciles it.
  if (state.settling) {
    return { kind: "settling" };
  }
  // Refunds reach the ledger only after the nightly export, so an unexported refund still counts as paid.
  if (state.refund && !state.refund.exported) {
    return { kind: "paid" };
  }
  // Loop over the lines and add up the amounts.
  let total = 0;
  for (const line of state.lines) {
    total += line.amount;
  }
  // TODO(PAY-431): show partial refunds once the ledger reports them.
  return { kind: "due", total };
}

export function receiptUrl(id: string) {
  // Existing receipts keep working after the move to the new provider.
  return `/receipts/${id}`;
}
