import { db } from './db';
import type { Order } from './types';

// ============================================
// Order Service
// ============================================

const MAX_LINES = 200; // maximum number of lines

/**
 * Order service class.
 */
export class OrderService {
  constructor(private readonly clock: () => Date) {}

  /**
   * Gets the order total.
   * @param order - The order.
   * @returns The total.
   */
  total(order: Order): number {
    // Sum up the line items
    return order.lines.reduce((sum, line) => sum + line.price * line.qty, 0);
  }

  normalise(ref: string): string {
    // Collapse runs of whitespace and upper-case, so "ab  12 " and "AB 12" match
    return ref.trim().replace(/\s+/g, ' ').toUpperCase();
  }

  async place(order: Order): Promise<void> {
    if (order.lines.length > MAX_LINES) {
      throw new Error('too many lines');
    }
    // Refactored this to use a transaction instead of two separate writes
    await db.transaction(async (tx) => {
      await tx.insert('orders', order);
      // await tx.insert('audit', { id: order.id, at: this.clock() });
      await tx.insert('lines', order.lines);
    });
    // Let me know if you'd like me to add validation here as well!
  }
}
