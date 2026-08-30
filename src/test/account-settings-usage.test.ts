import { describe, expect, it } from "vitest";
import { FREE_PLAN_CREDITS, usagePercentFromBalance } from "../components/account/AccountSettings";
import type { AccountBalance, AccountSubscription } from "../lib/tauri";

const freeSubscription: AccountSubscription = { subscribed: false };

function balance(overrides: Partial<AccountBalance> = {}): AccountBalance {
  return { usdMillis: 0, ...overrides };
}

describe("FREE_PLAN_CREDITS", () => {
  it("matches the current OS Accounts starting grant of 4,000", () => {
    expect(FREE_PLAN_CREDITS).toBe(4000);
  });
});

describe("usagePercentFromBalance", () => {
  it("uses usageRemainingPercent directly when present", () => {
    expect(
      usagePercentFromBalance(balance({ usageRemainingPercent: 42, credits: 999 }), {
        subscribed: true,
        plan: "pro",
        planCredits: 10_000,
      }),
    ).toBe(42);
  });

  it("clamps an out-of-range usageRemainingPercent to 0-100", () => {
    expect(usagePercentFromBalance(balance({ usageRemainingPercent: 150 }), freeSubscription)).toBe(
      100,
    );
    expect(usagePercentFromBalance(balance({ usageRemainingPercent: -5 }), freeSubscription)).toBe(
      0,
    );
  });

  it("derives the percentage from planCredits for a paid subscriber", () => {
    expect(
      usagePercentFromBalance(balance({ credits: 2_500 }), {
        subscribed: true,
        plan: "pro",
        planCredits: 5_000,
      }),
    ).toBe(50);
  });

  it("falls back to the free-plan denominator when a free payload omits usageRemainingPercent", () => {
    // A backward-compatible accounts payload: subscribed=false, credits present,
    // no usageRemainingPercent, no planCredits. The local denominator is the
    // only signal left, and it must match the current 4,000-credit grant.
    expect(usagePercentFromBalance(balance({ credits: 4_000 }), freeSubscription)).toBe(100);
    expect(usagePercentFromBalance(balance({ credits: 2_000 }), freeSubscription)).toBe(50);
    expect(usagePercentFromBalance(balance({ credits: 0 }), freeSubscription)).toBe(0);
  });

  it("ignores the free-plan denominator when subscription is not explicitly false", () => {
    // An absent subscription (distinct from { subscribed: false }) must not
    // trigger the free fallback, so a bare credits value never gets divided by
    // the 4,000-credit denominator.
    expect(usagePercentFromBalance(balance({ credits: 2_000, usdMillis: 0 }), undefined)).toBe(0);
    expect(
      usagePercentFromBalance(balance({ credits: 2_000, usdMillis: 0 }), { subscribed: true }),
    ).toBe(0);
  });

  it("maps usdMillis to 100 or 0 as a last resort", () => {
    expect(usagePercentFromBalance(balance({ usdMillis: 500 }), freeSubscription)).toBe(100);
    expect(usagePercentFromBalance(balance({ usdMillis: 0 }), freeSubscription)).toBe(0);
  });

  it("returns 0 when no balance signal is available", () => {
    expect(usagePercentFromBalance(undefined, undefined)).toBe(0);
    expect(usagePercentFromBalance(balance({ usdMillis: 0 }), freeSubscription)).toBe(0);
  });
});
