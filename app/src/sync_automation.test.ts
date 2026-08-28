import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  AUTOMATIC_SYNC_STORAGE_KEY,
  AutomaticSyncScheduler,
  readAutomaticSyncEnabled,
  writeAutomaticSyncEnabled,
} from "./sync_automation";

function scheduler(
  synchronize: () => Promise<void>,
  state: { online: boolean; visible: boolean },
  retryDelaysMs: readonly number[] = [10, 20],
): AutomaticSyncScheduler {
  return new AutomaticSyncScheduler(true, synchronize, {
    startupDelayMs: 1,
    debounceDelayMs: 5,
    retryDelaysMs,
    isOnline: () => state.online,
    isVisible: () => state.visible,
    setTimer: (callback, delayMs) => setTimeout(callback, delayMs),
    clearTimer: (timer) => clearTimeout(timer),
  });
}

beforeEach(() => {
  vi.useFakeTimers();
  localStorage.clear();
});

afterEach(() => {
  vi.useRealTimers();
});

describe("automatic synchronization preference", () => {
  it("defaults safely to manual mode and persists an explicit choice", () => {
    expect(readAutomaticSyncEnabled(localStorage)).toBe(false);
    writeAutomaticSyncEnabled(localStorage, true);
    expect(localStorage.getItem(AUTOMATIC_SYNC_STORAGE_KEY)).toBe("true");
    expect(readAutomaticSyncEnabled(localStorage)).toBe(true);
    localStorage.setItem(AUTOMATIC_SYNC_STORAGE_KEY, "invalid");
    expect(readAutomaticSyncEnabled(localStorage)).toBe(false);
  });

  it("falls back to manual mode when preference storage is unavailable", () => {
    const unavailable = {
      getItem: () => {
        throw new Error("denied");
      },
      setItem: () => {
        throw new Error("denied");
      },
    };
    expect(readAutomaticSyncEnabled(unavailable)).toBe(false);
    expect(() => writeAutomaticSyncEnabled(unavailable, true)).not.toThrow();
  });
});

describe("AutomaticSyncScheduler", () => {
  it("runs once after startup only while online and visible", async () => {
    const state = { online: false, visible: true };
    const synchronize = vi.fn(async () => undefined);
    const subject = scheduler(synchronize, state);
    subject.startup();
    await vi.advanceTimersByTimeAsync(100);
    expect(synchronize).not.toHaveBeenCalled();

    state.online = true;
    subject.networkBecameAvailable();
    await vi.advanceTimersByTimeAsync(1);
    expect(synchronize).toHaveBeenCalledOnce();
  });

  it("debounces consecutive local changes", async () => {
    const state = { online: true, visible: true };
    const synchronize = vi.fn(async () => undefined);
    const subject = scheduler(synchronize, state);
    subject.localChange();
    await vi.advanceTimersByTimeAsync(4);
    subject.localChange();
    await vi.advanceTimersByTimeAsync(4);
    expect(synchronize).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);
    expect(synchronize).toHaveBeenCalledOnce();
  });

  it("queues a local change made while synchronization is already running", async () => {
    const state = { online: true, visible: true };
    let finishFirstAttempt: (() => void) | undefined;
    let concurrentAttempts = 0;
    let maximumConcurrency = 0;
    const synchronize = vi.fn(async () => {
      concurrentAttempts += 1;
      maximumConcurrency = Math.max(maximumConcurrency, concurrentAttempts);
      if (synchronize.mock.calls.length === 1) {
        await new Promise<void>((resolve) => {
          finishFirstAttempt = resolve;
        });
      }
      concurrentAttempts -= 1;
    });
    const subject = scheduler(synchronize, state);
    subject.startup();
    await vi.advanceTimersByTimeAsync(1);
    expect(synchronize).toHaveBeenCalledOnce();

    subject.localChange();
    finishFirstAttempt?.();
    await Promise.resolve();
    await vi.advanceTimersByTimeAsync(4);
    expect(synchronize).toHaveBeenCalledOnce();
    await vi.advanceTimersByTimeAsync(1);
    expect(synchronize).toHaveBeenCalledTimes(2);
    expect(maximumConcurrency).toBe(1);
  });

  it("uses bounded retries and stops after the configured budget", async () => {
    const state = { online: true, visible: true };
    const synchronize = vi.fn(async () => {
      throw new Error("offline server");
    });
    const subject = scheduler(synchronize, state, [10, 20]);
    subject.startup();
    await vi.advanceTimersByTimeAsync(1);
    await vi.advanceTimersByTimeAsync(10);
    await vi.advanceTimersByTimeAsync(20);
    await vi.advanceTimersByTimeAsync(10_000);
    expect(synchronize).toHaveBeenCalledTimes(3);
  });

  it("cancels pending work in the background and resumes in foreground", async () => {
    const state = { online: true, visible: true };
    const synchronize = vi.fn(async () => undefined);
    const subject = scheduler(synchronize, state);
    subject.startup();
    state.visible = false;
    subject.visibilityChanged();
    await vi.advanceTimersByTimeAsync(100);
    expect(synchronize).not.toHaveBeenCalled();

    state.visible = true;
    subject.visibilityChanged();
    await vi.advanceTimersByTimeAsync(1);
    expect(synchronize).toHaveBeenCalledOnce();
  });

  it("disabling automatic mode cancels a pending attempt", async () => {
    const state = { online: true, visible: true };
    const synchronize = vi.fn(async () => undefined);
    const subject = scheduler(synchronize, state);
    subject.startup();
    subject.setEnabled(false);
    await vi.advanceTimersByTimeAsync(100);
    expect(synchronize).not.toHaveBeenCalled();
  });
});
