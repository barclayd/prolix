package com.acme.http;

import java.time.Duration;
import java.util.concurrent.ThreadLocalRandom;

/**
 * Retry policy.
 */
public final class RetryPolicy {
    // ===== DO NOT REORDER: ordinal() is persisted in the jobs table =====
    public enum Outcome { SUCCESS, RETRY, GIVE_UP }

    /**
     * Returns the delay before the given attempt.
     *
     * @param attempt 1-based attempt number; values below 1 are treated as 1
     * @return a full-jitter backoff, never more than 30 seconds
     */
    public Duration delay(int attempt) {
        // TODO(PAY-812): cap per tenant once plans have their own limits. A single global
        // cap is why the enterprise import job starves smaller tenants.
        long cap = Math.min(30_000L, 100L << Math.min(Math.max(attempt, 1), 16));
        return Duration.ofMillis(ThreadLocalRandom.current().nextLong(cap + 1));
    }

    // Callers should sleep for delay(attempt) rather than a fixed interval: fixed
    // sleeps synchronise clients and cause retry storms after an outage.
    public boolean shouldRetry(int status) {
        // 429 and 503 only; other 5xx responses are usually bugs, and retrying hides them
        return status == 429 || status == 503;
    }
}
