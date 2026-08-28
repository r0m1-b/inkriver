export const AUTOMATIC_SYNC_STORAGE_KEY = "inkriver.automaticSyncEnabled";

export interface BooleanPreferenceStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

export function readAutomaticSyncEnabled(storage: BooleanPreferenceStorage | null): boolean {
  if (!storage) return false;
  try {
    return storage.getItem(AUTOMATIC_SYNC_STORAGE_KEY) === "true";
  } catch {
    return false;
  }
}

export function writeAutomaticSyncEnabled(
  storage: BooleanPreferenceStorage | null,
  enabled: boolean,
): void {
  try {
    storage?.setItem(AUTOMATIC_SYNC_STORAGE_KEY, String(enabled));
  } catch {
    // A restricted WebView can deny access to localStorage. The in-memory
    // setting remains valid for the current session.
  }
}

export interface AutomaticSyncSchedulerOptions {
  startupDelayMs?: number;
  debounceDelayMs?: number;
  retryDelaysMs?: readonly number[];
  isOnline: () => boolean;
  isVisible: () => boolean;
  setTimer: (callback: () => void, delayMs: number) => ReturnType<typeof setTimeout>;
  clearTimer: (timer: ReturnType<typeof setTimeout>) => void;
}

const DEFAULT_STARTUP_DELAY_MS = 1_000;
const DEFAULT_DEBOUNCE_DELAY_MS = 5_000;
const DEFAULT_RETRY_DELAYS_MS = [30_000, 120_000, 600_000, 1_800_000] as const;

/**
 * Coordinates foreground-only automatic synchronization attempts.
 *
 * It intentionally knows nothing about Tauri or the DOM. The caller supplies
 * network/visibility probes and one synchronization callback, which keeps the
 * retry policy deterministic and testable without real network access.
 */
export class AutomaticSyncScheduler {
  private timer: ReturnType<typeof setTimeout> | null = null;
  private pendingDelayMs: number | null = null;
  private failures = 0;
  private running = false;
  private enabled: boolean;

  private readonly startupDelayMs: number;
  private readonly debounceDelayMs: number;
  private readonly retryDelaysMs: readonly number[];

  constructor(
    enabled: boolean,
    private readonly synchronize: () => Promise<void>,
    private readonly options: AutomaticSyncSchedulerOptions,
  ) {
    this.enabled = enabled;
    this.startupDelayMs = options.startupDelayMs ?? DEFAULT_STARTUP_DELAY_MS;
    this.debounceDelayMs = options.debounceDelayMs ?? DEFAULT_DEBOUNCE_DELAY_MS;
    this.retryDelaysMs = options.retryDelaysMs ?? DEFAULT_RETRY_DELAYS_MS;
  }

  setEnabled(enabled: boolean): void {
    this.enabled = enabled;
    this.failures = 0;
    this.cancelTimer();
    this.pendingDelayMs = null;
    if (enabled) this.schedule(this.startupDelayMs);
  }

  startup(): void {
    this.schedule(this.startupDelayMs);
  }

  localChange(): void {
    if (!this.enabled) return;
    this.failures = 0;
    this.cancelTimer();
    this.pendingDelayMs = null;
    this.schedule(this.debounceDelayMs);
  }

  networkBecameAvailable(): void {
    if (!this.enabled) return;
    this.failures = 0;
    this.cancelTimer();
    this.pendingDelayMs = null;
    this.schedule(this.startupDelayMs);
  }

  visibilityChanged(): void {
    if (!this.options.isVisible()) {
      this.cancelTimer();
      this.pendingDelayMs = null;
      return;
    }
    if (!this.enabled) return;
    this.failures = 0;
    this.schedule(this.startupDelayMs);
  }

  cancel(): void {
    this.cancelTimer();
    this.pendingDelayMs = null;
  }

  private schedule(delayMs: number): void {
    if (
      !this.enabled ||
      this.timer !== null ||
      !this.options.isOnline() ||
      !this.options.isVisible()
    ) return;
    if (this.running) {
      this.pendingDelayMs = delayMs;
      return;
    }
    this.timer = this.options.setTimer(() => {
      this.timer = null;
      void this.runAttempt();
    }, delayMs);
  }

  private cancelTimer(): void {
    if (this.timer === null) return;
    this.options.clearTimer(this.timer);
    this.timer = null;
  }

  private async runAttempt(): Promise<void> {
    if (
      !this.enabled ||
      this.running ||
      !this.options.isOnline() ||
      !this.options.isVisible()
    ) return;
    this.running = true;
    try {
      await this.synchronize();
      this.failures = 0;
    } catch {
      this.failures += 1;
    } finally {
      this.running = false;
      const pendingDelay = this.pendingDelayMs;
      this.pendingDelayMs = null;
      // Arm pending work or a retry only after releasing the running slot.
      const retryDelay = this.retryDelaysMs[this.failures - 1];
      if (pendingDelay !== null) {
        this.schedule(pendingDelay);
      } else if (this.failures > 0 && retryDelay !== undefined) {
        this.schedule(retryDelay);
      }
    }
  }
}
