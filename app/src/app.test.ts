import { createHash } from "node:crypto";
import { beforeEach, describe, expect, it, vi } from "vitest";
import tauriConfig from "../src-tauri/tauri.conf.json";
import {
  ARTICLE_BRIDGE_CSP_HASH,
  ARTICLE_BRIDGE_SCRIPT,
  InkRiverApp,
  articleSourceHost,
  buildArticleDocument,
  canOpenOriginal,
  detectPlatform,
  errorMessage,
  prepareArticleContent,
  readArticleTextSize,
  resolveArticleImageUrl,
  resolveExternalArticleUrl,
  writeArticleTextSize,
} from "./app";
import type { InkRiverApi } from "./api";
import type { ArticleDetail, ArticleSummary, Feed, RefreshReport } from "./types";

const summary: ArticleSummary = {
  id: "space::mars",
  feedId: "stable-feed-id",
  title: "Observer Mars au crépuscule",
  author: "Claire du Ciel",
  publishedAt: "2026-08-08T12:00:00Z",
  url: "https://space.example/mars",
  source: "substack",
  isRead: false,
  isFavorite: false,
};

const detail: ArticleDetail = {
  ...summary,
  content: "<p>Mars prend une teinte orangée.</p>",
  contentKind: "excerpt",
};

const secondSummary: ArticleSummary = {
  ...summary,
  id: "space::venus",
  title: "Observer Vénus à l'aube",
  isRead: true,
  isFavorite: true,
};

const secondDetail: ArticleDetail = {
  ...secondSummary,
  content: "<p>Vénus brille avant le lever du Soleil.</p>",
  contentKind: "full",
};

const feed: Feed = {
  id: "stable-feed-id",
  platform: "substack",
  url: "https://space.substack.com/feed",
  isActive: true,
  title: "Carnet du ciel",
  description: "Une lettre pour observer le ciel.",
  author: "Claire du Ciel",
  lastPublishedAt: "2026-08-08T12:00:00Z",
  lastSuccessAt: "2026-08-08T12:05:00Z",
  lastError: null,
  logoDataUrl: null,
};

const emptySyncRuntime = {
  lastAttemptAt: null,
  lastSuccessAt: null,
  lastError: null,
  lastReport: null,
};

beforeEach(() => {
  localStorage.clear();
});

function fakeApi(overrides: Partial<InkRiverApi> = {}): InkRiverApi {
  return {
    listArticles: vi.fn(async () => [structuredClone(summary)]),
    getArticle: vi.fn(async () => structuredClone(detail)),
    refreshFeeds: vi.fn(async (): Promise<RefreshReport> => ({
      activeFeeds: 1,
      collectedArticles: 1,
      insertedArticles: 0,
      updatedArticles: 1,
      autoArchivedArticles: 0,
      extractedArticles: 0,
      extractionFailedArticles: 0,
      extractionSkippedArticles: 0,
      errors: [],
    })),
    refreshFeed: vi.fn(async (): Promise<RefreshReport> => ({
      activeFeeds: 1,
      collectedArticles: 1,
      insertedArticles: 1,
      updatedArticles: 0,
      autoArchivedArticles: 0,
      extractedArticles: 0,
      extractionFailedArticles: 0,
      extractionSkippedArticles: 0,
      errors: [],
    })),
    setArticleRead: vi.fn(async () => undefined),
    setArticlesRead: vi.fn(async () => undefined),
    setArticleFavorite: vi.fn(async () => undefined),
    archiveArticle: vi.fn(async () => undefined),
    archiveArticles: vi.fn(async () => undefined),
    listFeeds: vi.fn(async () => [structuredClone(feed)]),
    addFeed: vi.fn(async () => structuredClone(feed)),
    setFeedActive: vi.fn(async () => structuredClone(feed)),
    deleteFeed: vi.fn(async () => ({ feedId: feed.id, deletedArticles: 1 })),
    syncPairingStatus: vi.fn(async () => ({
      ...emptySyncRuntime,
      configured: false,
      webdavBaseUrl: null,
      webdavUsername: null,
      keyId: null,
      devices: [],
    })),
    configureSyncGroup: vi.fn(async () => ({
      ...emptySyncRuntime,
      configured: true,
      webdavBaseUrl: "https://cloud.example/dav/inkriver",
      webdavUsername: "alice",
      keyId: "key-123",
      devices: [{ deviceId: "linux", displayName: "Linux", isLocal: true, revokedAt: null }],
    })),
    pairingInvitation: vi.fn(async () => ({
      invitation: "inkriver://pair/example",
      qrCodeDataUrl: "data:image/svg+xml;base64,PHN2Zz48L3N2Zz4=",
    })),
    joinSyncGroup: vi.fn(async () => ({
      ...emptySyncRuntime,
      configured: true,
      webdavBaseUrl: "https://cloud.example/dav/inkriver",
      webdavUsername: "alice",
      keyId: "key-123",
      devices: [{ deviceId: "android", displayName: "Android", isLocal: true, revokedAt: null }],
    })),
    renameSyncDevice: vi.fn(async () => ({
      ...emptySyncRuntime,
      configured: true,
      webdavBaseUrl: "https://cloud.example/dav/inkriver",
      webdavUsername: "alice",
      keyId: "key-123",
      devices: [{ deviceId: "linux", displayName: "Portable", isLocal: true, revokedAt: null }],
    })),
    revokeSyncDevice: vi.fn(async () => ({
      ...emptySyncRuntime,
      configured: true,
      webdavBaseUrl: "https://cloud.example/dav/inkriver",
      webdavUsername: "alice",
      keyId: "key-123",
      devices: [{ deviceId: "android", displayName: "Android", isLocal: false, revokedAt: "2026-08-28T10:00:00Z" }],
    })),
    synchronizeNow: vi.fn(async () => ({
      uploadedSegments: 0,
      reusedSegments: 0,
      exportedEvents: 0,
      downloadedSegments: 0,
      receivedEvents: 0,
      importedEvents: 0,
      duplicateEvents: 0,
      appliedEvents: 0,
      pendingEvents: 0,
    })),
    deleteSyncConfiguration: vi.fn(async () => ({
      ...emptySyncRuntime,
      configured: false,
      webdavBaseUrl: null,
      webdavUsername: null,
      keyId: null,
      devices: [],
    })),
    ...overrides,
  };
}

async function flush(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 0));
}

async function flushMicrotasks(): Promise<void> {
  for (let index = 0; index < 8; index += 1) await Promise.resolve();
}

function dispatchTouch(
  target: HTMLElement,
  type: "touchstart" | "touchmove" | "touchend" | "touchcancel",
  clientX = 0,
  clientY = 0,
): Event {
  const event = new Event(type, { bubbles: true, cancelable: true });
  const touches = type === "touchend" || type === "touchcancel"
    ? []
    : [{ clientX, clientY }];
  Object.defineProperty(event, "touches", { configurable: true, value: touches });
  target.dispatchEvent(event);
  return event;
}

function installMobileViewport(): () => void {
  const originalMatchMedia = window.matchMedia;
  Object.defineProperty(window, "matchMedia", {
    configurable: true,
    value: vi.fn((media: string) => ({
      matches: media === "(max-width: 720px)",
      media,
    }) as MediaQueryList),
  });
  return () => {
    Object.defineProperty(window, "matchMedia", {
      configurable: true,
      value: originalMatchMedia,
    });
  };
}

function longPressArticle(root: HTMLElement, articleId: string): void {
  const article = root.querySelector<HTMLElement>(`[data-article-id="${articleId}"]`)!;
  dispatchTouch(article, "touchstart", 50, 80);
  vi.advanceTimersByTime(500);
  expect(dispatchTouch(article, "touchend").defaultPrevented).toBe(true);
}

async function mounted(
  api = fakeApi(),
  opener = vi.fn(async () => undefined),
  confirmer = vi.fn(() => true),
  scanner: (() => Promise<string>) | null = null,
) {
  document.body.innerHTML = '<div id="app"></div>';
  const root = document.querySelector<HTMLElement>("#app")!;
  const app = new InkRiverApp(root, api, opener, confirmer, scanner);
  const initialization = app.init();
  expect(root.querySelector('[data-testid="loading"]')).not.toBeNull();
  await initialization;
  return { root, app, api, opener, confirmer };
}

describe("InkRiverApp", () => {
  it("renders the InkRiver logo in the application header", async () => {
    const { root } = await mounted();
    const logo = root.querySelector<HTMLImageElement>(".brand-logo");

    expect(logo?.getAttribute("src")).toBe("/inkriver-logo.png");
    expect(logo?.getAttribute("alt")).toBe("");
    expect(root.querySelector(".brand small")?.textContent).toBe("All your feeds. One flow.");
  });

  it("provides compact mobile subscription actions and a back button", async () => {
    const restoreViewport = installMobileViewport();
    try {
      const { root, api } = await mounted();
      const addButton = root.querySelector<HTMLButtonElement>(".mobile-add-subscription")!;
      const settingsButton = root.querySelector<HTMLButtonElement>(".mobile-settings")!;

      expect(addButton.getAttribute("aria-label")).toBe("Ajouter un abonnement");
      expect(addButton.querySelector("svg")).not.toBeNull();
      expect(settingsButton.getAttribute("aria-label")).toBe("Gestion des abonnements");
      expect(settingsButton.querySelector("svg")).not.toBeNull();

      addButton.click();
      const form = root.querySelector<HTMLFormElement>("#feed-form")!;
      root.querySelector<HTMLInputElement>('input[name="url"]')!.value = feed.url;
      form.dispatchEvent(new Event("submit", { cancelable: true }));
      await flush();

      expect(api.addFeed).toHaveBeenCalledWith(feed.url, "other");
      expect(api.refreshFeed).toHaveBeenCalledWith(feed.id);
      expect(root.querySelector("main")?.classList).toContain("articles-view");

      root.querySelector<HTMLElement>(".mobile-settings")!.click();
      const mobileToolbar = root.querySelector<HTMLElement>(".mobile-feed-topbar")!;
      expect(mobileToolbar.textContent?.trim()).toBe("Gestion des abonnements");
      const back = mobileToolbar.querySelector<HTMLButtonElement>('[data-action="show-articles"]')!;
      expect(back.getAttribute("aria-label")).toBe("Retour aux articles");
      expect(back.querySelector("svg")).not.toBeNull();
      back.click();
      expect(root.querySelector("main")?.classList).toContain("articles-view");
    } finally {
      restoreViewport();
    }
  });

  it("renders the cached timeline without refreshing on startup", async () => {
    const { root, api } = await mounted();
    expect(root.textContent).toContain("Observer Mars au crépuscule");
    expect(api.listArticles).toHaveBeenCalledOnce();
    expect(api.refreshFeeds).not.toHaveBeenCalled();
  });

  it("uses separate timeline and reader screens for the responsive layout", async () => {
    const { root, app, api } = await mounted();

    expect(root.querySelector("main")?.classList).toContain("mobile-timeline");
    root.querySelector<HTMLElement>('[data-action="select-article"]')!.click();
    await flush();

    expect(api.getArticle).toHaveBeenCalledWith(summary.id);
    expect(root.querySelector("main")?.classList).toContain("mobile-reader");
    const back = root.querySelector<HTMLButtonElement>(
      '[data-action="mobile-reader-back"]',
    );
    expect(back?.textContent?.trim()).toBe("");
    expect(back?.getAttribute("aria-label")).toBe("Retour aux articles");
    expect(back?.getAttribute("title")).toBe("Retour aux articles");
    expect(back?.querySelector("svg")).not.toBeNull();
    const mobileActions = root.querySelector<HTMLElement>(".mobile-reader-actions")!;
    expect(mobileActions.closest(".mobile-reader-toolbar")).not.toBeNull();
    expect(mobileActions.querySelector('[data-action="toggle-read"]')).not.toBeNull();
    expect(mobileActions.querySelector('[data-action="favorite"]')).not.toBeNull();
    expect(mobileActions.querySelector('[data-action="archive-article"]')).not.toBeNull();
    mobileActions.querySelector<HTMLElement>('[data-action="favorite"]')!.click();
    await flush();
    expect(api.setArticleFavorite).toHaveBeenCalledWith(summary.id, true);

    expect(app.handleBackNavigation()).toBe(true);
    expect(root.querySelector("main")?.classList).toContain("mobile-timeline");
    expect(root.querySelector(`[data-article-row-id="${summary.id}"]`)).not.toBeNull();
    expect(app.handleBackNavigation()).toBe(false);
  });

  it("uses medium article text without zoom controls on mobile", async () => {
    const restoreViewport = installMobileViewport();
    localStorage.setItem("inkriver.articleTextSize", "large");
    try {
      const { root } = await mounted();
      root.querySelector<HTMLElement>('[data-action="select-article"]')!.click();
      await flush();

      const mobileActions = root.querySelector<HTMLElement>(".mobile-reader-actions")!;
      expect(mobileActions.querySelector('[data-action="decrease-text-size"]')).toBeNull();
      expect(mobileActions.querySelector('[data-action="increase-text-size"]')).toBeNull();
      expect(root.querySelector<HTMLIFrameElement>(".article-content")?.srcdoc).toContain(
        'style="--article-font-size:18px"',
      );
    } finally {
      restoreViewport();
    }
  });

  it("navigates through the active mobile list with bottom buttons and swipes", async () => {
    const restoreViewport = installMobileViewport();
    const api = fakeApi({
      listArticles: vi.fn(async () => [structuredClone(summary), structuredClone(secondSummary)]),
      getArticle: vi.fn(async (articleId) =>
        structuredClone(articleId === secondDetail.id ? secondDetail : detail),
      ),
    });
    try {
      const { root, app } = await mounted(api);
      expect(root.querySelector(".mobile-reader-navigation")).toBeNull();

      root.querySelector<HTMLElement>('[data-article-id="space::mars"]')!.click();
      await flush();

      let navigation = root.querySelector<HTMLElement>(".mobile-reader-navigation")!;
      expect(navigation.getAttribute("aria-label")).toBe("Navigation entre les articles");
      expect(navigation.querySelector<HTMLButtonElement>('[data-action="reader-previous"]')?.disabled)
        .toBe(true);
      expect(navigation.querySelector<HTMLButtonElement>('[data-action="reader-next"]')?.disabled)
        .toBe(false);

      navigation.querySelector<HTMLButtonElement>('[data-action="reader-next"]')!.click();
      await flush();
      expect(root.querySelector(".reader-article h1")?.textContent).toBe(secondSummary.title);
      navigation = root.querySelector<HTMLElement>(".mobile-reader-navigation")!;
      expect(navigation.querySelector<HTMLButtonElement>('[data-action="reader-previous"]')?.disabled)
        .toBe(false);
      expect(navigation.querySelector<HTMLButtonElement>('[data-action="reader-next"]')?.disabled)
        .toBe(true);

      let reader = root.querySelector<HTMLElement>(".reader")!;
      dispatchTouch(reader, "touchstart", 200, 300);
      expect(dispatchTouch(reader, "touchmove", 320, 305).defaultPrevented).toBe(true);
      dispatchTouch(reader, "touchend");
      await flush();
      expect(root.querySelector(".reader-article h1")?.textContent).toBe(summary.title);

      const frame = root.querySelector<HTMLIFrameElement>(".article-content")!;
      window.dispatchEvent(new MessageEvent("message", {
        data: { type: "inkriver:article-swipe", direction: "next" },
        source: frame.contentWindow,
      }));
      await flush();
      expect(root.querySelector(".reader-article h1")?.textContent).toBe(secondSummary.title);

      expect(app.handleBackNavigation()).toBe(true);
      root.querySelector<HTMLElement>('[data-article-view="favorites"]')!.click();
      root.querySelector<HTMLElement>('[data-article-id="space::venus"]')!.click();
      await flush();
      navigation = root.querySelector<HTMLElement>(".mobile-reader-navigation")!;
      expect(navigation.querySelectorAll("button:disabled")).toHaveLength(2);
    } finally {
      restoreViewport();
    }
  });

  it("enters mobile multi-selection after a stationary long press", async () => {
    const restoreViewport = installMobileViewport();
    try {
      const api = fakeApi({
        listArticles: vi.fn(async () => [
          structuredClone(summary),
          structuredClone(secondSummary),
        ]),
      });
      const { root } = await mounted(api);
      vi.useFakeTimers();

      longPressArticle(root, summary.id);

      expect(api.getArticle).not.toHaveBeenCalled();
      expect(root.querySelector(".timeline-view-tabs")).toBeNull();
      expect(root.querySelector(".article-selection-toolbar")?.textContent).toContain("1");
      const firstRow = root.querySelector<HTMLElement>(
        `[data-article-row-id="${summary.id}"]`,
      )!;
      expect(firstRow.classList).toContain("multi-selected");
      expect(firstRow.querySelector(".article-selection-check svg")).not.toBeNull();
      expect(firstRow.querySelector('[data-action="select-article"]')?.getAttribute("aria-pressed"))
        .toBe("true");
      expect(firstRow.querySelector(".article-row-actions")).not.toBeNull();

      root.querySelector<HTMLElement>(`[data-article-id="${secondSummary.id}"]`)!.click();
      expect(root.querySelectorAll(".article-row.multi-selected")).toHaveLength(2);
      root.querySelector<HTMLElement>(`[data-article-id="${summary.id}"]`)!.click();
      expect(root.querySelectorAll(".article-row.multi-selected")).toHaveLength(1);
      root.querySelector<HTMLElement>(`[data-article-id="${secondSummary.id}"]`)!.click();
      expect(root.querySelector(".article-selection-toolbar")).toBeNull();
      expect(root.querySelector(".timeline-view-tabs")).not.toBeNull();
    } finally {
      vi.useRealTimers();
      restoreViewport();
    }
  });

  it("cancels the mobile long press when the finger moves", async () => {
    const restoreViewport = installMobileViewport();
    try {
      const { root, api } = await mounted();
      vi.useFakeTimers();
      const article = root.querySelector<HTMLElement>(`[data-article-id="${summary.id}"]`)!;

      dispatchTouch(article, "touchstart", 50, 80);
      dispatchTouch(article, "touchmove", 50, 91);
      vi.advanceTimersByTime(500);
      dispatchTouch(article, "touchend");

      expect(root.querySelector(".article-selection-toolbar")).toBeNull();
      expect(api.getArticle).not.toHaveBeenCalled();
    } finally {
      vi.useRealTimers();
      restoreViewport();
    }
  });

  it("selects visible articles and applies grouped read states", async () => {
    const restoreViewport = installMobileViewport();
    try {
      const api = fakeApi({
        listArticles: vi.fn(async () => [
          structuredClone(summary),
          structuredClone(secondSummary),
        ]),
      });
      const { root } = await mounted(api);
      vi.useFakeTimers();

      longPressArticle(root, summary.id);
      root.querySelector<HTMLInputElement>('[data-action="toggle-all-articles"]')!.click();
      expect(root.querySelectorAll(".article-row.multi-selected")).toHaveLength(2);
      root.querySelector<HTMLElement>('[data-action="mark-selected-read"]')!.click();
      await flushMicrotasks();

      expect(api.setArticlesRead).toHaveBeenCalledWith(
        expect.arrayContaining([summary.id, secondSummary.id]),
        true,
      );
      expect(root.querySelector(".article-selection-toolbar")).toBeNull();
      expect(root.querySelectorAll(".article-row.read")).toHaveLength(2);

      longPressArticle(root, summary.id);
      root.querySelector<HTMLInputElement>('[data-action="toggle-all-articles"]')!.click();
      root.querySelector<HTMLElement>('[data-action="mark-selected-unread"]')!.click();
      await flushMicrotasks();
      expect(api.setArticlesRead).toHaveBeenLastCalledWith(
        expect.arrayContaining([summary.id, secondSummary.id]),
        false,
      );
      expect(root.querySelectorAll(".article-row.unread")).toHaveLength(2);
    } finally {
      vi.useRealTimers();
      restoreViewport();
    }
  });

  it("limits select all to the current filter and confirms grouped archiving", async () => {
    const restoreViewport = installMobileViewport();
    try {
      const api = fakeApi({
        listArticles: vi.fn(async () => [
          structuredClone(summary),
          structuredClone(secondSummary),
        ]),
      });
      const { root } = await mounted(api);
      root.querySelector<HTMLElement>('[data-article-view="favorites"]')!.click();
      vi.useFakeTimers();

      longPressArticle(root, secondSummary.id);
      const selectAll = root.querySelector<HTMLInputElement>(
        '[data-action="toggle-all-articles"]',
      )!;
      expect(selectAll.checked).toBe(true);
      expect(root.querySelectorAll(".article-row.multi-selected")).toHaveLength(1);
      root.querySelector<HTMLElement>('[data-action="archive-selected"]')!.click();
      expect(root.querySelector(".archive-confirmation")?.textContent).toContain(
        "Archiver 1 article",
      );
      root.querySelector<HTMLElement>('[data-action="confirm-archive"]')!.click();
      await flushMicrotasks();

      expect(api.archiveArticles).toHaveBeenCalledWith([secondSummary.id]);
      expect(root.querySelector(`[data-article-row-id="${secondSummary.id}"]`)).toBeNull();
      expect(root.querySelector(".article-selection-toolbar")).toBeNull();
    } finally {
      vi.useRealTimers();
      restoreViewport();
    }
  });

  it("keeps mobile selection after a grouped update failure", async () => {
    const restoreViewport = installMobileViewport();
    try {
      const api = fakeApi({
        setArticlesRead: vi.fn(async () =>
          Promise.reject({ code: "storage", message: "Mise à jour impossible" })
        ),
      });
      const { root } = await mounted(api);
      vi.useFakeTimers();

      longPressArticle(root, summary.id);
      root.querySelector<HTMLElement>('[data-action="mark-selected-read"]')!.click();
      await flushMicrotasks();

      expect(root.querySelector(".article-selection-toolbar")).not.toBeNull();
      expect(root.querySelector(".article-row.multi-selected")).not.toBeNull();
      expect(root.querySelector('[role="alert"]')?.textContent).toContain(
        "Mise à jour impossible",
      );
    } finally {
      vi.useRealTimers();
      restoreViewport();
    }
  });

  it("locks mobile selection controls during a grouped update", async () => {
    const restoreViewport = installMobileViewport();
    let finishUpdate: (() => void) | undefined;
    try {
      const api = fakeApi({
        setArticlesRead: vi.fn(
          () => new Promise<void>((resolve) => {
            finishUpdate = resolve;
          }),
        ),
      });
      const { root } = await mounted(api);
      vi.useFakeTimers();

      longPressArticle(root, summary.id);
      root.querySelector<HTMLElement>('[data-action="mark-selected-read"]')!.click();

      expect(
        Array.from(
          root.querySelectorAll<HTMLInputElement | HTMLButtonElement>(
            ".article-selection-toolbar input, .article-selection-toolbar button",
          ),
        ).every((control) => control.disabled),
      ).toBe(true);
      root.querySelector<HTMLElement>(`[data-article-id="${summary.id}"]`)!.click();
      expect(root.querySelector(".article-row.multi-selected")).not.toBeNull();

      finishUpdate?.();
      await flushMicrotasks();
      expect(root.querySelector(".article-selection-toolbar")).toBeNull();
    } finally {
      vi.useRealTimers();
      restoreViewport();
    }
  });

  it("keeps mobile selection when grouped archiving fails", async () => {
    const restoreViewport = installMobileViewport();
    try {
      const api = fakeApi({
        archiveArticles: vi.fn(async () =>
          Promise.reject({ code: "storage", message: "Archivage groupé impossible" })
        ),
      });
      const { root } = await mounted(api);
      vi.useFakeTimers();

      longPressArticle(root, summary.id);
      root.querySelector<HTMLElement>('[data-action="archive-selected"]')!.click();
      root.querySelector<HTMLElement>('[data-action="confirm-archive"]')!.click();
      await flushMicrotasks();

      expect(root.querySelector(".article-selection-toolbar")).not.toBeNull();
      expect(root.querySelector(".article-row.multi-selected")).not.toBeNull();
      expect(root.querySelector(`[data-article-row-id="${summary.id}"]`)).not.toBeNull();
      expect(root.querySelector('[role="alert"]')?.textContent).toContain(
        "Archivage groupé impossible",
      );
    } finally {
      vi.useRealTimers();
      restoreViewport();
    }
  });

  it("cancels mobile selection with Back and disables conflicting gestures", async () => {
    const restoreViewport = installMobileViewport();
    try {
      const { root, app, api } = await mounted();
      vi.useFakeTimers();
      longPressArticle(root, summary.id);

      const timeline = root.querySelector<HTMLElement>(".timeline")!;
      dispatchTouch(timeline, "touchstart", 24, 100);
      dispatchTouch(timeline, "touchmove", 24, 260);
      dispatchTouch(timeline, "touchend");
      const row = root.querySelector<HTMLElement>(`[data-article-row-id="${summary.id}"]`)!;
      const foreground = row.querySelector<HTMLElement>(".article-row-foreground")!;
      Object.defineProperty(row, "getBoundingClientRect", {
        configurable: true,
        value: () => ({ left: 0, width: 320 }),
      });
      dispatchTouch(foreground, "touchstart", 50, 100);
      dispatchTouch(foreground, "touchmove", 220, 102);
      dispatchTouch(foreground, "touchend");

      expect(api.refreshFeeds).not.toHaveBeenCalled();
      expect(api.archiveArticle).not.toHaveBeenCalled();
      expect(app.handleBackNavigation()).toBe(true);
      expect(root.querySelector(".article-selection-toolbar")).toBeNull();

      longPressArticle(root, summary.id);
      root.querySelector<HTMLElement>('[data-action="subscriptions"]')!.click();
      expect(root.querySelector(".feed-management")).not.toBeNull();
      root.querySelector<HTMLElement>('[data-action="show-articles"]')!.click();
      expect(root.querySelector(".article-selection-toolbar")).toBeNull();
    } finally {
      vi.useRealTimers();
      restoreViewport();
    }
  });

  it("closes mobile overlays before leaving the current screen", async () => {
    const { root, app } = await mounted();
    root.querySelector<HTMLElement>('[data-action="select-article"]')!.click();
    await flush();
    root.querySelector<HTMLElement>('[data-action="archive-article"]')!.click();

    expect(root.querySelector(".archive-confirmation")).not.toBeNull();
    expect(app.handleBackNavigation()).toBe(true);
    expect(root.querySelector(".archive-confirmation")).toBeNull();
    expect(root.querySelector("main")?.classList).toContain("mobile-reader");

    root.querySelector<HTMLElement>('[data-action="subscriptions"]')!.click();
    root.querySelector<HTMLElement>('[data-action="add-subscription"]')!.click();
    expect(root.querySelector(".add-subscription")).not.toBeNull();
    expect(app.handleBackNavigation()).toBe(true);
    expect(root.querySelector(".add-subscription")).toBeNull();
    expect(root.querySelector("main")?.classList).toContain("feeds-view");
    expect(app.handleBackNavigation()).toBe(true);
    expect(root.querySelector("main")?.classList).toContain("mobile-timeline");
  });

  it("renders the full InkRiver wordmark while no article is selected", async () => {
    const { root } = await mounted();
    const logo = root.querySelector<HTMLImageElement>(".reader-placeholder-logo");

    expect(logo?.getAttribute("src")).toBe("/inkriver-wordmark.png");
    expect(logo?.getAttribute("alt")).toBe("InkRiver");
    expect(root.querySelector(".reader-placeholder")?.textContent).toContain(
      "Sélectionnez un article dans la chronologie.",
    );
  });

  it("renders refresh as an accessible icon-only action", async () => {
    const { root } = await mounted();
    const refresh = root.querySelector<HTMLButtonElement>('[data-action="refresh"]')!;

    expect(refresh.textContent?.trim()).toBe("");
    expect(refresh.getAttribute("title")).toBe("Actualiser");
    expect(refresh.getAttribute("aria-label")).toBe("Actualiser");
    expect(refresh.getAttribute("aria-busy")).toBe("false");
    expect(refresh.querySelector("svg")).not.toBeNull();
  });

  it("refreshes by pulling down only from the top of the mobile timeline", async () => {
    const originalMatchMedia = window.matchMedia;
    Object.defineProperty(window, "matchMedia", {
      configurable: true,
      value: vi.fn((media: string) => ({
        matches: media === "(max-width: 720px)",
        media,
      }) as MediaQueryList),
    });
    try {
      const { root, api } = await mounted();
      let timeline = root.querySelector<HTMLElement>(".timeline")!;
      const indicator = timeline.querySelector<HTMLElement>("[data-pull-refresh]")!;

      expect(indicator.getAttribute("aria-hidden")).toBe("true");
      dispatchTouch(timeline, "touchstart", 24, 100);
      const move = dispatchTouch(timeline, "touchmove", 24, 260);
      expect(move.defaultPrevented).toBe(true);
      expect(indicator.classList).toContain("ready");
      expect(indicator.textContent).toContain("Relâchez pour actualiser");

      dispatchTouch(timeline, "touchend");
      expect(api.refreshFeeds).toHaveBeenCalledOnce();
      const refreshingIndicator = root.querySelector<HTMLElement>("[data-pull-refresh]")!;
      expect(refreshingIndicator.classList).toContain("refreshing");
      expect(refreshingIndicator.textContent?.trim()).toBe("");
      expect(refreshingIndicator.getAttribute("role")).toBe("status");
      expect(refreshingIndicator.getAttribute("aria-label")).toBe("Actualisation en cours");
      expect(root.querySelector(".shell")?.getAttribute("aria-busy")).toBe("true");
      expect(root.querySelector("[data-app-interaction-lock]")).not.toBeNull();
      root.querySelector<HTMLElement>(".mobile-settings")!.click();
      expect(root.querySelector("main")?.classList).toContain("articles-view");
      await flush();

      expect(root.querySelector(".shell")?.hasAttribute("aria-busy")).toBe(false);
      expect(root.querySelector("[data-app-interaction-lock]")).toBeNull();

      timeline = root.querySelector<HTMLElement>(".timeline")!;
      timeline.scrollTop = 10;
      dispatchTouch(timeline, "touchstart", 24, 100);
      dispatchTouch(timeline, "touchmove", 24, 280);
      dispatchTouch(timeline, "touchend");
      expect(api.refreshFeeds).toHaveBeenCalledOnce();
    } finally {
      Object.defineProperty(window, "matchMedia", {
        configurable: true,
        value: originalMatchMedia,
      });
    }
  });

  it("shows only cached favorites in the favorites tab and opens them", async () => {
    const api = fakeApi({
      listArticles: vi.fn(async () => [structuredClone(summary), structuredClone(secondSummary)]),
      getArticle: vi.fn(async () => structuredClone(secondDetail)),
    });
    const { root } = await mounted(api);

    root.querySelector<HTMLElement>('[data-article-view="favorites"]')!.click();

    expect(root.querySelector('[data-article-view="favorites"]')?.getAttribute("aria-selected")).toBe("true");
    expect(root.textContent).not.toContain("Observer Mars au crépuscule");
    expect(root.textContent).toContain("Observer Vénus à l'aube");
    expect(api.listArticles).toHaveBeenCalledOnce();
    expect(api.refreshFeeds).not.toHaveBeenCalled();

    root.querySelector<HTMLElement>('[data-article-id="space::venus"]')!.click();
    await flush();
    expect(api.getArticle).toHaveBeenCalledWith("space::venus");
    expect(root.querySelector(".reader-article")?.textContent).toContain("Observer Vénus à l'aube");
  });

  it("renders an explicit empty state when no article is favorite", async () => {
    const { root } = await mounted();

    root.querySelector<HTMLElement>('[data-article-view="favorites"]')!.click();

    expect(root.querySelector('[data-testid="favorites-empty"]')?.textContent).toContain(
      "Aucun article favori",
    );
    expect(root.querySelector("[data-article-row-id]")).toBeNull();
  });

  it("switches exclusively between all, favorites, and unread without calling the backend", async () => {
    const api = fakeApi({
      listArticles: vi.fn(async () => [structuredClone(summary), structuredClone(secondSummary)]),
    });
    const { root } = await mounted(api);
    const timeline = root.querySelector<HTMLElement>(".timeline")!;
    const activeViews = () =>
      Array.from(root.querySelectorAll('[data-action="article-view"]'))
        .filter((button) => button.getAttribute("aria-selected") === "true")
        .map((button) => (button as HTMLElement).dataset.articleView);

    expect(activeViews()).toEqual(["all"]);
    expect(root.querySelector('[data-article-view="unread"]')?.textContent).toContain("1");
    expect(root.querySelector('[data-article-row-id="space::mars"]')).not.toBeNull();
    expect(root.querySelector('[data-article-row-id="space::venus"]')).not.toBeNull();

    timeline.scrollTop = 320;
    root.querySelector<HTMLElement>('[data-article-view="unread"]')!.click();
    expect(activeViews()).toEqual(["unread"]);
    expect(root.querySelector<HTMLElement>(".timeline")?.scrollTop).toBe(0);
    expect(root.querySelector('[data-article-row-id="space::mars"]')).not.toBeNull();
    expect(root.querySelector('[data-article-row-id="space::venus"]')).toBeNull();

    root.querySelector<HTMLElement>('[data-article-view="favorites"]')!.click();
    expect(activeViews()).toEqual(["favorites"]);
    expect(root.querySelector('[data-article-row-id="space::mars"]')).toBeNull();
    expect(root.querySelector('[data-article-row-id="space::venus"]')).not.toBeNull();

    root.querySelector<HTMLElement>('[data-article-view="all"]')!.click();
    expect(activeViews()).toEqual(["all"]);
    expect(root.querySelector('[data-article-row-id="space::mars"]')).not.toBeNull();
    expect(root.querySelector('[data-article-row-id="space::venus"]')).not.toBeNull();
    expect(api.listArticles).toHaveBeenCalledOnce();
    expect(api.refreshFeeds).not.toHaveBeenCalled();
  });

  it("renders an explicit empty state when no article is unread", async () => {
    const api = fakeApi({
      listArticles: vi.fn(async () => [structuredClone(secondSummary)]),
    });
    const { root } = await mounted(api);

    root.querySelector<HTMLElement>('[data-article-view="unread"]')!.click();
    expect(root.querySelector('[data-testid="unread-empty"]')?.textContent).toContain(
      "Aucun article non lu",
    );
    expect(root.querySelector('[data-testid="unread-empty"]')?.textContent).toContain(
      "Sélectionnez « Tous »",
    );
    expect(root.querySelector("[data-article-row-id]")).toBeNull();
  });

  it("keeps an opened article visible when it leaves the unread list", async () => {
    const { root, api } = await mounted();
    root.querySelector<HTMLElement>('[data-article-view="unread"]')!.click();
    root.querySelector<HTMLElement>('[data-article-id="space::mars"]')!.click();
    await flush();

    expect(api.setArticleRead).toHaveBeenCalledWith("space::mars", true);
    expect(root.querySelector('[data-article-row-id="space::mars"]')).toBeNull();
    expect(root.querySelector('[data-testid="unread-empty"]')).not.toBeNull();
    expect(root.querySelector(".reader-article")?.textContent).toContain(
      "Observer Mars au crépuscule",
    );
    expect(
      root.querySelector('[data-article-view="unread"]')?.getAttribute("aria-selected"),
    ).toBe("true");
  });

  it("immediately adds and removes the open article in the favorites view", async () => {
    const api = fakeApi({
      listArticles: vi.fn(async () => [structuredClone(summary), structuredClone(secondSummary)]),
    });
    const { root } = await mounted(api);
    root.querySelector<HTMLElement>('[data-article-id="space::mars"]')!.click();
    await flush();
    root.querySelector<HTMLElement>('[data-article-view="favorites"]')!.click();

    expect(root.querySelector('[data-article-row-id="space::mars"]')).toBeNull();
    root.querySelector<HTMLElement>('[data-action="favorite"]')!.click();
    await flush();
    expect(root.querySelector('[data-article-row-id="space::mars"]')).not.toBeNull();
    expect(root.querySelector('[data-article-view="favorites"]')?.textContent).toContain("2");

    root.querySelector<HTMLElement>('[data-action="favorite"]')!.click();
    await flush();
    expect(root.querySelector('[data-article-row-id="space::mars"]')).toBeNull();
    expect(root.querySelector('[data-article-view="favorites"]')?.textContent).toContain("1");
    expect(root.querySelector(".reader-article")?.textContent).toContain("Observer Mars au crépuscule");
  });

  it("renders accessible quick actions whose icons represent the current states", async () => {
    const api = fakeApi({
      listArticles: vi.fn(async () => [structuredClone(summary), structuredClone(secondSummary)]),
    });
    const { root } = await mounted(api);
    const firstRow = root.querySelector<HTMLElement>(
      '[data-article-row-id="space::mars"]',
    )!;
    const secondRow = root.querySelector<HTMLElement>(
      '[data-article-row-id="space::venus"]',
    )!;

    expect(firstRow.querySelector('[data-action="timeline-favorite"]')?.getAttribute("aria-pressed")).toBe("false");
    expect(firstRow.querySelector('[data-action="timeline-favorite"]')?.getAttribute("aria-label")).toContain("Ajouter aux favoris");
    expect(firstRow.querySelector('[data-action="timeline-read"]')?.getAttribute("aria-pressed")).toBe("false");
    expect(firstRow.querySelector('[data-action="timeline-read"]')?.getAttribute("aria-label")).toContain("Marquer comme lu");
    expect(secondRow.querySelector('[data-action="timeline-favorite"]')?.getAttribute("aria-pressed")).toBe("true");
    expect(secondRow.querySelector('[data-action="timeline-read"]')?.getAttribute("aria-pressed")).toBe("true");
    expect(firstRow.querySelector('[data-action="timeline-archive"]')?.getAttribute("aria-label")).toContain("Observer Mars au crépuscule");
    expect(firstRow.querySelectorAll("button")).toHaveLength(4);
  });

  it("places a header-sized logo beside the author while leaving the title full-width", async () => {
    const { root } = await mounted();
    const row = root.querySelector<HTMLElement>('[data-article-row-id="space::mars"]')!;
    const logo = row.querySelector<HTMLElement>(".article-list-logo .source-logo");
    const copy = row.querySelector<HTMLElement>(".article-list-copy")!;
    const title = copy.querySelector<HTMLElement>(":scope > strong")!;

    expect(logo).not.toBeNull();
    expect(row.querySelector(".row-top .source-identity")).not.toBeNull();
    expect(row.querySelector(".article-select > .article-list-logo")).toBeNull();
    expect(row.querySelector(".article-list-source > .article-list-logo + .byline")?.textContent).toBe("Claire du Ciel");
    expect(copy.querySelector(".row-top .byline")?.textContent).toBe("Claire du Ciel");
    expect(copy.querySelector(".row-top time")).not.toBeNull();
    expect(title.textContent).toBe("Observer Mars au crépuscule");
    expect(title.getAttribute("title")).toBe("Observer Mars au crépuscule");
  });

  it("renders platform icons without redundant source labels", async () => {
    const mediumArticle = {
      ...structuredClone(summary),
      id: "source::medium",
      source: "medium" as const,
    };
    const rssArticle = {
      ...structuredClone(summary),
      id: "source::rss",
      source: "other" as const,
    };
    const api = fakeApi({
      listArticles: vi.fn(async () => [
        structuredClone(summary),
        mediumArticle,
        rssArticle,
      ]),
    });
    const { root } = await mounted(api);

    expect(root.querySelector('[data-source-icon="substack"]')).not.toBeNull();
    expect(root.querySelector('[data-source-icon="medium"]')).not.toBeNull();
    expect(root.querySelector('[data-source-icon="rss"]')).not.toBeNull();
    expect(root.querySelector('[data-source-icon="medium"] path')?.getAttribute("d")).toMatch(
      /^M4\.21 0A4\.201/,
    );
    expect(root.querySelector('[data-source-icon="substack"] path')?.getAttribute("d")).toBe(
      "M22.539 8.242H1.46V5.406h21.08v2.836zM1.46 10.812V24L12 18.11 22.54 24V10.812H1.46zM22.54 0H1.46v2.836h21.08V0z",
    );
    expect(root.querySelector('[data-source-icon="medium"]')?.closest(".source-logo")).not.toBeNull();
    expect(root.querySelector('[data-source-icon="medium"]')?.closest(".source")).toBeNull();
    expect(root.querySelector('[data-article-row-id="source::medium"] .source')).toBeNull();
    expect(root.querySelector('[data-article-row-id="source::rss"] .source')).toBeNull();
    expect(root.querySelector('[data-source-icon="substack"]')?.getAttribute("aria-hidden")).toBe(
      "true",
    );
  });

  it("uses a cached website logo for Other feeds in every view", async () => {
    const logoDataUrl = "data:image/png;base64,iVBORw0KGgo=";
    const otherFeed: Feed = {
      ...structuredClone(feed),
      platform: "other",
      logoDataUrl,
    };
    const otherSummary: ArticleSummary = {
      ...structuredClone(summary),
      source: "other",
    };
    const otherDetail: ArticleDetail = {
      ...structuredClone(detail),
      source: "other",
    };
    const api = fakeApi({
      listFeeds: vi.fn(async () => [otherFeed]),
      listArticles: vi.fn(async () => [otherSummary]),
      getArticle: vi.fn(async () => otherDetail),
    });
    const { root } = await mounted(api);

    expect(
      root.querySelector<HTMLImageElement>('[data-article-row-id="space::mars"] [data-feed-logo]')
        ?.src,
    ).toBe(logoDataUrl);

    root.querySelector<HTMLElement>('[data-action="select-article"]')!.click();
    await flush();
    expect(root.querySelector<HTMLImageElement>(".reader-article [data-feed-logo]")?.src).toBe(
      logoDataUrl,
    );

    root.querySelector<HTMLElement>('[data-action="subscriptions"]')!.click();
    expect(root.querySelector<HTMLImageElement>(".feed-card [data-feed-logo]")?.src).toBe(
      logoDataUrl,
    );
  });

  it("keeps branded icons and falls back to RSS for absent, unsafe, or broken logos", async () => {
    const feeds: Feed[] = [
      { ...structuredClone(feed), logoDataUrl: "data:image/png;base64,ignored" },
      {
        ...structuredClone(feed),
        id: "other-feed",
        platform: "other",
        logoDataUrl: "data:image/png;base64,broken",
      },
      {
        ...structuredClone(feed),
        id: "unsafe-feed",
        platform: "other",
        logoDataUrl: "https://tracker.example/favicon.png",
      },
      {
        ...structuredClone(feed),
        id: "medium-feed",
        platform: "medium",
        logoDataUrl: "data:image/png;base64,ignored",
      },
    ];
    const articles: ArticleSummary[] = [
      structuredClone(summary),
      { ...structuredClone(summary), id: "other", feedId: "other-feed", source: "other" },
      { ...structuredClone(summary), id: "unsafe", feedId: "unsafe-feed", source: "other" },
      { ...structuredClone(summary), id: "medium", feedId: "medium-feed", source: "medium" },
    ];
    const { root } = await mounted(fakeApi({
      listFeeds: vi.fn(async () => feeds),
      listArticles: vi.fn(async () => articles),
    }));

    expect(root.querySelector('[data-article-row-id="space::mars"] [data-source-icon="substack"]')).not.toBeNull();
    expect(root.querySelector('[data-article-row-id="space::mars"] [data-feed-logo]')).toBeNull();
    expect(root.querySelector('[data-article-row-id="medium"] [data-source-icon="medium"]')).not.toBeNull();
    expect(root.querySelector('[data-article-row-id="medium"] [data-feed-logo]')).toBeNull();
    expect(root.querySelector('[data-article-row-id="unsafe"] [data-feed-logo]')).toBeNull();
    expect(root.querySelector('[data-article-row-id="unsafe"] [data-source-icon="rss"]')).not.toBeNull();

    const broken = root.querySelector<HTMLImageElement>(
      '[data-article-row-id="other"] [data-feed-logo]',
    )!;
    broken.dispatchEvent(new Event("error"));
    expect(root.querySelector('[data-article-row-id="other"] [data-feed-logo]')).toBeNull();
    expect(root.querySelector('[data-article-row-id="other"] [data-source-icon="rss"]')).not.toBeNull();
  });

  it("toggles a favorite from the timeline without opening the article", async () => {
    const { root, api } = await mounted();

    root.querySelector<HTMLElement>('[data-action="timeline-favorite"]')!.click();
    await flush();

    expect(api.setArticleFavorite).toHaveBeenCalledWith("space::mars", true);
    expect(api.getArticle).not.toHaveBeenCalled();
    const button = root.querySelector<HTMLElement>('[data-action="timeline-favorite"]')!;
    expect(button.getAttribute("aria-pressed")).toBe("true");
    expect(button.getAttribute("title")).toBe("Retirer des favoris");

    button.click();
    await flush();
    expect(api.setArticleFavorite).toHaveBeenNthCalledWith(2, "space::mars", false);
    expect(root.querySelector('[data-action="timeline-favorite"]')?.getAttribute("aria-pressed")).toBe("false");
  });

  it("preserves timeline scroll position across action renders", async () => {
    const { root } = await mounted();
    root.querySelector<HTMLElement>(".timeline")!.scrollTop = 420;

    root.querySelector<HTMLElement>('[data-action="timeline-favorite"]')!.click();
    expect(root.querySelector<HTMLElement>(".timeline")!.scrollTop).toBe(420);
    await flush();

    expect(root.querySelector<HTMLElement>(".timeline")!.scrollTop).toBe(420);
  });

  it("scrolls a newly selected article into view with the nearest alignment", async () => {
    const scrollIntoView = vi.fn();
    const previousScrollIntoView = Object.getOwnPropertyDescriptor(
      HTMLElement.prototype,
      "scrollIntoView",
    );
    Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
      configurable: true,
      value: scrollIntoView,
      writable: true,
    });
    try {
      const { root } = await mounted();

      root.querySelector<HTMLElement>("[data-article-id]")!.click();
      await flush();

      expect(scrollIntoView).toHaveBeenCalledOnce();
      expect(scrollIntoView).toHaveBeenCalledWith({ block: "nearest", inline: "nearest" });
    } finally {
      if (previousScrollIntoView) {
        Object.defineProperty(
          HTMLElement.prototype,
          "scrollIntoView",
          previousScrollIntoView,
        );
      } else {
        delete (HTMLElement.prototype as Partial<HTMLElement>).scrollIntoView;
      }
    }
  });

  it("toggles read state from the timeline without opening the article", async () => {
    const { root, api } = await mounted();

    root.querySelector<HTMLElement>('[data-action="timeline-read"]')!.click();
    await flush();

    expect(api.setArticleRead).toHaveBeenCalledWith("space::mars", true);
    expect(api.getArticle).not.toHaveBeenCalled();
    expect(root.querySelector('[data-action="timeline-read"]')?.getAttribute("aria-pressed")).toBe("true");
    expect(root.querySelector("[data-article-row-id]")?.classList).toContain("read");

    root.querySelector<HTMLElement>('[data-action="timeline-read"]')!.click();
    await flush();
    expect(api.setArticleRead).toHaveBeenNthCalledWith(2, "space::mars", false);
    expect(root.querySelector('[data-action="timeline-read"]')?.getAttribute("aria-pressed")).toBe("false");
  });

  it("keeps other quick actions available while a timeline update is pending", async () => {
    let finishUpdate: (() => void) | undefined;
    const api = fakeApi({
      setArticleRead: vi.fn(
        () => new Promise<void>((resolve) => {
          finishUpdate = resolve;
        }),
      ),
    });
    const { root } = await mounted(api);

    root.querySelector<HTMLElement>('[data-action="timeline-read"]')!.click();
    const pendingRead = root.querySelector<HTMLButtonElement>('[data-action="timeline-read"]')!;
    const favorite = root.querySelector<HTMLButtonElement>('[data-action="timeline-favorite"]')!;
    expect(pendingRead.disabled).toBe(true);
    expect(pendingRead.getAttribute("aria-busy")).toBe("true");
    expect(favorite.disabled).toBe(false);
    pendingRead.click();
    expect(api.setArticleRead).toHaveBeenCalledOnce();

    finishUpdate!();
    await flush();
    expect(root.querySelector('[data-action="timeline-read"]')?.getAttribute("aria-pressed")).toBe("true");
  });

  it("keeps quick-action state unchanged when persistence fails", async () => {
    const api = fakeApi({
      setArticleFavorite: vi.fn(async () =>
        Promise.reject({ code: "storage", message: "Favori non enregistré" }),
      ),
    });
    const { root } = await mounted(api);

    root.querySelector<HTMLElement>('[data-action="timeline-favorite"]')!.click();
    await flush();

    expect(root.querySelector('[role="alert"]')?.textContent).toContain("Favori non enregistré");
    expect(root.querySelector('[data-action="timeline-favorite"]')?.getAttribute("aria-pressed")).toBe("false");
    expect(api.getArticle).not.toHaveBeenCalled();
  });

  it("renders an empty state that opens the add-subscription dialog", async () => {
    const { root } = await mounted(fakeApi({ listArticles: vi.fn(async () => []) }));
    expect(root.querySelector('[data-testid="empty"]')).not.toBeNull();
    root.querySelector<HTMLElement>('[data-testid="empty"] [data-action="add-subscription"]')!.click();
    expect(root.querySelector('[role="dialog"]')).not.toBeNull();
    expect(root.querySelector('[role="dialog"] [data-action="toggle-feed"]')).toBeNull();
  });

  it("keeps a fatal startup error visible", async () => {
    const { root } = await mounted(
      fakeApi({ listArticles: vi.fn(async () => Promise.reject({ code: "storage", message: "Base indisponible" })) }),
    );
    expect(root.querySelector('[role="alert"]')?.textContent).toContain("Base indisponible");
  });

  it("loads a selected article and marks it as read", async () => {
    const { root, api } = await mounted();
    root.querySelector<HTMLElement>("[data-article-id]")!.click();
    await flush();
    expect(api.getArticle).toHaveBeenCalledWith("space::mars");
    expect(api.setArticleRead).toHaveBeenCalledWith("space::mars", true);

    const readButton = root.querySelector<HTMLElement>('[data-action="toggle-read"]')!;
    expect(readButton.textContent?.trim()).toBe("");
    expect(readButton.getAttribute("title")).toBe("Marquer comme non lu");
    expect(readButton.getAttribute("aria-label")).toBe("Marquer comme non lu");
    expect(readButton.getAttribute("aria-pressed")).toBe("true");
    expect(readButton.querySelector("svg")).not.toBeNull();
    expect(root.querySelector("[data-article-row-id]")?.classList).toContain("read");
    expect(root.querySelector<HTMLIFrameElement>(".article-content")?.srcdoc).toContain(
      "Mars prend une teinte orangée",
    );
    expect(root.querySelector(".mobile-reader-navigation")).toBeNull();
  });

  it("does not write the read state when opening an article that is already read", async () => {
    const readSummary = { ...summary, isRead: true };
    const api = fakeApi({
      listArticles: vi.fn(async () => [structuredClone(readSummary)]),
      getArticle: vi.fn(async () => ({ ...structuredClone(detail), isRead: true })),
    });
    const { root } = await mounted(api);

    root.querySelector<HTMLElement>("[data-article-id]")!.click();
    await flush();

    expect(api.setArticleRead).not.toHaveBeenCalled();
  
    const readButton = root.querySelector<HTMLElement>('[data-action="toggle-read"]')!;
    expect(readButton.getAttribute("title")).toBe("Marquer comme non lu");
    expect(readButton.getAttribute("aria-label")).toBe("Marquer comme non lu");
  });

  it("marks the selected article unread and read again in both panels", async () => {
    const { root, api } = await mounted();
    root.querySelector<HTMLElement>("[data-article-id]")!.click();
    await flush();

    root.querySelector<HTMLElement>('[data-action="toggle-read"]')!.click();
    await flush();
    expect(api.setArticleRead).toHaveBeenNthCalledWith(2, "space::mars", false);

    const readButton = root.querySelector<HTMLElement>('[data-action="toggle-read"]')!;
    expect(readButton.getAttribute("title")).toBe("Marquer comme lu");
    expect(readButton.getAttribute("aria-label")).toBe("Marquer comme lu");
    expect(root.querySelector('[data-action="toggle-read"]')?.getAttribute("title")).toBe(
      "Marquer comme lu",
    );
    expect(root.querySelector('[data-action="toggle-read"]')?.getAttribute("aria-pressed")).toBe("false");
    expect(root.querySelector("[data-article-row-id]")?.classList).toContain("unread");

    root.querySelector<HTMLElement>('[data-action="toggle-read"]')!.click();
    await flush();
    expect(api.setArticleRead).toHaveBeenNthCalledWith(3, "space::mars", true);
  
    expect(root.querySelector("[data-article-row-id]")?.classList).toContain("read");
  });

  it("keeps a selected article unread until it is reopened after another selection", async () => {
    const getArticle = vi.fn(async (articleId: string) =>
      articleId === secondDetail.id
        ? structuredClone(secondDetail)
        : structuredClone(detail),
    );
    const api = fakeApi({
      listArticles: vi.fn(async () => [structuredClone(summary), structuredClone(secondSummary)]),
      getArticle,
    });
    const { root } = await mounted(api);

    root.querySelector<HTMLElement>('[data-article-id="space::mars"]')!.click();
    await flush();
    root.querySelector<HTMLElement>(
      '[data-article-row-id="space::mars"] [data-action="timeline-read"]',
    )!.click();
    await flush();

    const readButton = root.querySelector<HTMLElement>('[data-action="toggle-read"]')!;
    expect(readButton.getAttribute("title")).toBe("Marquer comme lu");
    expect(readButton.getAttribute("aria-label")).toBe("Marquer comme lu");
    
    root.querySelector<HTMLElement>('[data-article-id="space::mars"]')!.click();
    await flush();
    expect(getArticle).toHaveBeenCalledTimes(1);
    expect(api.setArticleRead).toHaveBeenCalledTimes(2);
  
    expect(readButton.getAttribute("title")).toBe("Marquer comme lu");
    expect(readButton.getAttribute("aria-label")).toBe("Marquer comme lu");

    root.querySelector<HTMLElement>('[data-article-id="space::venus"]')!.click();
    await flush();
    root.querySelector<HTMLElement>('[data-article-id="space::mars"]')!.click();
    await flush();

    expect(getArticle).toHaveBeenCalledTimes(3);
    expect(api.setArticleRead).toHaveBeenNthCalledWith(3, "space::mars", true);
  });

  it("disables the read action while its update is pending", async () => {
    let finishUpdate: (() => void) | undefined;
    const api = fakeApi({
      listArticles: vi.fn(async () => [{ ...structuredClone(summary), isRead: true }]),
      getArticle: vi.fn(async () => ({ ...structuredClone(detail), isRead: true })),
      setArticleRead: vi.fn(
        () => new Promise<void>((resolve) => {
          finishUpdate = resolve;
        }),
      ),
    });
    const { root } = await mounted(api);
    root.querySelector<HTMLElement>("[data-article-id]")!.click();
    await flush();

    root.querySelector<HTMLElement>('[data-action="toggle-read"]')!.click();
    const pendingButton = root.querySelector<HTMLButtonElement>('[data-action="toggle-read"]')!;
    expect(pendingButton.disabled).toBe(true);
    expect(pendingButton.textContent?.trim()).toBe("");
    expect(pendingButton.getAttribute("aria-busy")).toBe("true");
    expect(pendingButton.getAttribute("title")).toBe("Marquer comme non lu");

    finishUpdate!();
    await flush();

    const readButton = root.querySelector<HTMLElement>('[data-action="toggle-read"]')!;
    expect(readButton.getAttribute("title")).toBe("Marquer comme lu");
    expect(readButton.getAttribute("aria-label")).toBe("Marquer comme lu");
  });

  it("keeps the previous read state when the explicit update fails", async () => {
    const api = fakeApi({
      listArticles: vi.fn(async () => [{ ...structuredClone(summary), isRead: true }]),
      getArticle: vi.fn(async () => ({ ...structuredClone(detail), isRead: true })),
      setArticleRead: vi.fn(async () =>
        Promise.reject({ code: "storage", message: "État de lecture non enregistré" }),
      ),
    });
    const { root } = await mounted(api);
    root.querySelector<HTMLElement>("[data-article-id]")!.click();
    await flush();

    root.querySelector<HTMLElement>('[data-action="toggle-read"]')!.click();
    await flush();

    expect(api.setArticleRead).toHaveBeenCalledWith("space::mars", false);
    expect(root.querySelector('[role="alert"]')?.textContent).toContain(
      "État de lecture non enregistré",
    );
    const readButton = root.querySelector<HTMLElement>('[data-action="toggle-read"]')!;
    expect(readButton.getAttribute("title")).toBe("Marquer comme non lu");
    expect(readButton.getAttribute("aria-label")).toBe("Marquer comme non lu");
    expect(root.querySelector("[data-article-row-id]")?.classList).toContain("read");
  });

  it("toggles a favorite from the reading panel", async () => {
    const { root, api } = await mounted();
    root.querySelector<HTMLElement>("[data-article-id]")!.click();
    await flush();
    root.querySelector<HTMLElement>('[data-action="favorite"]')!.click();
    await flush();
    expect(api.setArticleFavorite).toHaveBeenCalledWith("space::mars", true);
    const favoriteButton = root.querySelector<HTMLElement>('[data-action="favorite"]')!;
    expect(favoriteButton.textContent?.trim()).toBe("");
    expect(favoriteButton.getAttribute("title")).toBe("Retirer des favoris");
    expect(favoriteButton.getAttribute("aria-label")).toBe("Retirer des favoris");
    expect(favoriteButton.getAttribute("aria-pressed")).toBe("true");
    expect(favoriteButton.querySelector("svg")).not.toBeNull();
    expect(root.querySelector('[data-action="timeline-favorite"]')?.getAttribute("aria-pressed")).toBe("true");

    root.querySelector<HTMLElement>('[data-action="timeline-favorite"]')!.click();
    await flush();
    expect(api.setArticleFavorite).toHaveBeenNthCalledWith(2, "space::mars", false);
    expect(root.querySelector('[data-action="favorite"]')?.getAttribute("title")).toBe(
      "Ajouter aux favoris",
    );
  });

  it("keeps only the source action at the end of the article", async () => {
    const { root, opener } = await mounted();
    root.querySelector<HTMLElement>("[data-article-id]")!.click();
    await flush();

    const footer = root.querySelector<HTMLElement>(".reader-footer")!;
    const source = footer.querySelector<HTMLButtonElement>('[data-action="open-source"]')!;
    expect(footer.getAttribute("aria-label")).toBe("Actions de fin d’article");
    expect(footer.querySelectorAll(":scope > button")).toHaveLength(1);
    expect(footer.querySelector('[data-action="favorite"]')).toBeNull();
    expect(footer.querySelector('[data-action="archive-article"]')).toBeNull();
    expect(source.getAttribute("title")).toBe(
      "Ouvrir le lien : https://space.example/mars",
    );
    expect(source.querySelector("svg")).not.toBeNull();

    source.click();
    await flush();
    expect(opener).toHaveBeenCalledWith("https://space.example/mars");
    expect(root.querySelector(".reader-article")).not.toBeNull();
  });

  it("disables the footer source action when the article URL is unsupported", async () => {
    const unsupportedDetail = { ...structuredClone(detail), url: "mailto:reader@example.com" };
    const api = fakeApi({ getArticle: vi.fn(async () => unsupportedDetail) });
    const { root, opener } = await mounted(api);
    root.querySelector<HTMLElement>("[data-article-id]")!.click();
    await flush();

    const source = root.querySelector<HTMLButtonElement>(
      '.reader-footer [data-action="open-source"]',
    )!;
    expect(source.disabled).toBe(true);
    expect(source.getAttribute("title")).toBe("Lien source indisponible");
    source.click();
    expect(opener).not.toHaveBeenCalled();
  });

  it("preserves the reading position when opening the source from the footer", async () => {
    const { root, opener } = await mounted();
    root.querySelector<HTMLElement>("[data-article-id]")!.click();
    await flush();

    let reader = root.querySelector<HTMLElement>(".reader")!;
    root.querySelector<HTMLIFrameElement>(".article-content")!.style.height = "2400px";
    reader.scrollTop = 900;
    root.querySelector<HTMLElement>('.reader-footer [data-action="open-source"]')!.click();
    await flush();

    reader = root.querySelector<HTMLElement>(".reader")!;
    expect(opener).toHaveBeenCalledWith("https://space.example/mars");
    expect(reader.scrollTop).toBe(900);
  });

  it("persists the text size across articles and application instances", async () => {
    const firstApi = fakeApi({
      listArticles: vi.fn(async () => [structuredClone(summary), structuredClone(secondSummary)]),
      getArticle: vi.fn(async (articleId) =>
        structuredClone(articleId === secondDetail.id ? secondDetail : detail),
      ),
    });
    let mountedApp = await mounted(firstApi);
    mountedApp.root.querySelector<HTMLElement>('[data-article-id="space::mars"]')!.click();
    await flush();
    mountedApp.root.querySelector<HTMLButtonElement>(
      '[data-action="increase-text-size"]',
    )!.click();

    mountedApp.root.querySelector<HTMLElement>('[data-article-id="space::venus"]')!.click();
    await flush();
    expect(
      mountedApp.root.querySelector<HTMLIFrameElement>(".article-content")?.srcdoc,
    ).toContain('style="--article-font-size:22px"');

    mountedApp = await mounted();
    mountedApp.root.querySelector<HTMLElement>("[data-article-id]")!.click();
    await flush();
    expect(
      mountedApp.root.querySelector<HTMLIFrameElement>(".article-content")?.srcdoc,
    ).toContain('style="--article-font-size:22px"');
  });

  it("restores relative reading progress after text reflow", async () => {
    const { root } = await mounted();
    root.querySelector<HTMLElement>("[data-article-id]")!.click();
    await flush();

    const reader = root.querySelector<HTMLElement>(".reader")!;
    const frame = root.querySelector<HTMLIFrameElement>(".article-content")!;
    Object.defineProperty(reader, "clientHeight", { configurable: true, value: 500 });
    Object.defineProperty(reader, "scrollHeight", { configurable: true, value: 2500 });
    reader.scrollTop = 1000;

    root.querySelector<HTMLButtonElement>('[data-action="increase-text-size"]')!.click();
    Object.defineProperty(reader, "scrollHeight", { configurable: true, value: 3500 });
    window.dispatchEvent(
      new MessageEvent("message", {
        data: { type: "inkriver:article-height", height: 3000 },
        source: frame.contentWindow,
      }),
    );
    expect(frame.style.height).toBe("3000px");
    expect(reader.scrollTop).toBe(1500);

    Object.defineProperty(reader, "scrollHeight", { configurable: true, value: 4500 });
    window.dispatchEvent(
      new MessageEvent("message", {
        data: { type: "inkriver:article-height", height: 4000 },
        source: frame.contentWindow,
      }),
    );
    expect(reader.scrollTop).toBe(1500);
  });

  it("opens an accessible archive dialog and honors cancellation", async () => {
    const confirmer = vi.fn(() => true);
    const mountedApp = await mounted(fakeApi(), undefined, confirmer);
    mountedApp.root.querySelector<HTMLElement>("[data-article-id]")!.click();
    await flush();

    const archive = mountedApp.root.querySelector<HTMLButtonElement>(
      '[data-action="archive-article"]',
    )!;
    expect(archive.textContent?.trim()).toBe("");
    expect(archive.getAttribute("title")).toBe("Archiver l’article");
    expect(archive.getAttribute("aria-label")).toBe("Archiver l’article");
    expect(archive.querySelector("svg")).not.toBeNull();
    archive.click();

    const dialog = mountedApp.root.querySelector<HTMLElement>(".archive-confirmation")!;
    expect(dialog.getAttribute("role")).toBe("dialog");
    expect(dialog.getAttribute("aria-modal")).toBe("true");
    expect(dialog.textContent).toContain("Observer Mars au crépuscule");
    expect(dialog.textContent).toContain("ne pourra pas être restauré");
    expect(document.activeElement?.getAttribute("data-action")).toBe("cancel-archive");
    expect(confirmer).not.toHaveBeenCalled();
    expect(mountedApp.api.archiveArticle).not.toHaveBeenCalled();

    dialog.querySelector<HTMLElement>('[data-action="cancel-archive"]')!.click();
    expect(mountedApp.root.querySelector(".archive-confirmation")).toBeNull();
    expect(mountedApp.api.archiveArticle).not.toHaveBeenCalled();
    expect(mountedApp.root.querySelector(".reader-article")).not.toBeNull();
    expect(document.activeElement?.getAttribute("data-action")).toBe("archive-article");
  });

  it("closes the archive dialog with Escape", async () => {
    const { root, api } = await mounted();
    root.querySelector<HTMLElement>("[data-article-id]")!.click();
    await flush();
    root.querySelector<HTMLElement>('[data-action="archive-article"]')!.click();

    root.querySelector<HTMLElement>(".archive-confirmation")!.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
    );

    expect(root.querySelector(".archive-confirmation")).toBeNull();
    expect(api.archiveArticle).not.toHaveBeenCalled();
  });

  it("archives the selected article after confirmation", async () => {
    const { root, api, confirmer } = await mounted();
    root.querySelector<HTMLElement>("[data-article-id]")!.click();
    await flush();

    root.querySelector<HTMLElement>('[data-action="archive-article"]')!.click();
    root.querySelector<HTMLElement>('[data-action="confirm-archive"]')!.click();
    await flush();

    expect(confirmer).not.toHaveBeenCalled();
    expect(api.archiveArticle).toHaveBeenCalledWith("space::mars");
    expect(root.querySelector('[data-article-row-id="space::mars"]')).toBeNull();
    expect(root.querySelector(".reader-placeholder")).not.toBeNull();
    expect(root.querySelector('[role="status"]')?.textContent).toContain("Article archivé");

    const dismiss = root.querySelector<HTMLButtonElement>('[data-action="dismiss-notice"]')!;
    expect(dismiss.getAttribute("aria-label")).toBe("Fermer la notification");
    expect(dismiss.getAttribute("title")).toBe("Fermer la notification");
    dismiss.click();
    expect(root.querySelector(".banner.notice")?.classList).toContain("is-leaving");
    await new Promise((resolve) => setTimeout(resolve, 180));
    expect(root.querySelector('[role="status"]')).toBeNull();
  });

  it("opens the archive confirmation from the timeline without opening the article", async () => {
    const { root, api } = await mounted();
    const archive = root.querySelector<HTMLButtonElement>(
      '[data-action="timeline-archive"]',
    )!;

    expect(archive.getAttribute("data-article-id")).toBe(summary.id);
    expect(archive.getAttribute("aria-label")).toContain(summary.title);
    archive.click();

    expect(root.querySelector(".reader-placeholder")).not.toBeNull();
    expect(root.querySelector(".archive-confirmation")?.textContent).toContain(summary.title);
    expect(api.getArticle).not.toHaveBeenCalled();
    expect(api.setArticleRead).not.toHaveBeenCalled();

    root.querySelector<HTMLElement>('[data-action="cancel-archive"]')!.click();
    expect(root.querySelector(".archive-confirmation")).toBeNull();
    expect(document.activeElement?.getAttribute("data-action")).toBe("timeline-archive");
    expect(document.activeElement?.getAttribute("data-article-id")).toBe(summary.id);
  });

  it("archives a mobile timeline row after a deliberate half-width swipe", async () => {
    const originalMatchMedia = window.matchMedia;
    Object.defineProperty(window, "matchMedia", {
      configurable: true,
      value: vi.fn((media: string) => ({
        matches: media === "(max-width: 720px)",
        media,
      }) as MediaQueryList),
    });
    try {
      const { root, api, confirmer } = await mounted();
      const row = root.querySelector<HTMLElement>('[data-article-row-id="space::mars"]')!;
      const foreground = row.querySelector<HTMLElement>(".article-row-foreground")!;
      const favorite = row.querySelector<HTMLElement>('[data-action="timeline-favorite"]')!;
      Object.defineProperty(row, "getBoundingClientRect", {
        configurable: true,
        value: () => ({ left: 0, width: 320 }),
      });

      dispatchTouch(foreground, "touchstart", 10, 100);
      dispatchTouch(foreground, "touchmove", 230, 102);
      dispatchTouch(foreground, "touchend");
      expect(api.archiveArticle).not.toHaveBeenCalled();

      dispatchTouch(favorite, "touchstart", 50, 100);
      dispatchTouch(favorite, "touchmove", 230, 102);
      dispatchTouch(favorite, "touchend");
      expect(api.archiveArticle).not.toHaveBeenCalled();

      dispatchTouch(foreground, "touchstart", 50, 100);
      dispatchTouch(foreground, "touchmove", 185, 103);
      dispatchTouch(foreground, "touchend");
      expect(api.archiveArticle).not.toHaveBeenCalled();
      expect(foreground.style.transform).toBe("");

      dispatchTouch(foreground, "touchstart", 50, 100);
      const move = dispatchTouch(foreground, "touchmove", 220, 103);
      expect(move.defaultPrevented).toBe(true);
      expect(row.classList).toContain("swipe-ready");
      expect(row.textContent).toContain("Relâchez pour archiver");
      dispatchTouch(foreground, "touchend");

      expect(api.archiveArticle).toHaveBeenCalledWith(summary.id);
      expect(confirmer).not.toHaveBeenCalled();
      expect(row.classList).toContain("swipe-committing");
      foreground.dispatchEvent(new Event("transitionend"));
      await flushMicrotasks();

      expect(root.querySelector('[data-article-row-id="space::mars"]')).toBeNull();
      expect(root.querySelector('[role="status"]')?.textContent).toContain("Article archivé");
    } finally {
      Object.defineProperty(window, "matchMedia", {
        configurable: true,
        value: originalMatchMedia,
      });
    }
  });

  it("restores a swiped mobile row when archiving fails", async () => {
    const originalMatchMedia = window.matchMedia;
    Object.defineProperty(window, "matchMedia", {
      configurable: true,
      value: vi.fn((media: string) => ({
        matches: media === "(max-width: 720px)",
        media,
      }) as MediaQueryList),
    });
    try {
      const api = fakeApi({
        archiveArticle: vi.fn(async () =>
          Promise.reject({ code: "storage", message: "Archivage impossible" }),
        ),
      });
      const { root } = await mounted(api);
      const row = root.querySelector<HTMLElement>('[data-article-row-id="space::mars"]')!;
      const foreground = row.querySelector<HTMLElement>(".article-row-foreground")!;
      Object.defineProperty(row, "getBoundingClientRect", {
        configurable: true,
        value: () => ({ left: 0, width: 320 }),
      });

      dispatchTouch(foreground, "touchstart", 50, 100);
      dispatchTouch(foreground, "touchmove", 220, 102);
      dispatchTouch(foreground, "touchend");
      foreground.dispatchEvent(new Event("transitionend"));
      await flushMicrotasks();

      expect(root.querySelector('[data-article-row-id="space::mars"]')).not.toBeNull();
      expect(root.querySelector('[role="alert"]')?.textContent).toContain("Archivage impossible");
      expect(root.querySelector(".archive-confirmation")).toBeNull();
    } finally {
      Object.defineProperty(window, "matchMedia", {
        configurable: true,
        value: originalMatchMedia,
      });
    }
  });

  it("archives a timeline article without closing a different selected article", async () => {
    const api = fakeApi({
      listArticles: vi.fn(async () => [
        structuredClone(summary),
        structuredClone(secondSummary),
      ]),
    });
    const { root } = await mounted(api);
    root.querySelector<HTMLElement>(`[data-article-id="${summary.id}"]`)!.click();
    await flush();

    root.querySelector<HTMLElement>(
      `[data-article-row-id="${secondSummary.id}"] [data-action="timeline-archive"]`,
    )!.click();
    expect(root.querySelector(".archive-confirmation")?.textContent).toContain(
      secondSummary.title,
    );
    root.querySelector<HTMLElement>('[data-action="confirm-archive"]')!.click();
    await flush();

    expect(api.archiveArticle).toHaveBeenCalledWith(secondSummary.id);
    expect(root.querySelector(`[data-article-row-id="${secondSummary.id}"]`)).toBeNull();
    expect(root.querySelector(`[data-article-row-id="${summary.id}"]`)?.classList).toContain(
      "selected",
    );
    expect(root.querySelector(".reader-article")?.textContent).toContain(summary.title);
  });

  it("keeps a timeline article when its archive request fails", async () => {
    const api = fakeApi({
      archiveArticle: vi.fn(async () =>
        Promise.reject({ code: "storage", message: "Archivage impossible" }),
      ),
    });
    const { root } = await mounted(api);

    root.querySelector<HTMLElement>('[data-action="timeline-archive"]')!.click();
    root.querySelector<HTMLElement>('[data-action="confirm-archive"]')!.click();
    await flush();

    expect(root.querySelector('[role="alert"]')?.textContent).toContain("Archivage impossible");
    expect(root.querySelector(`[data-article-row-id="${summary.id}"]`)).not.toBeNull();
    expect(root.querySelector(".reader-placeholder")).not.toBeNull();
  });

  it("keeps the selected article visible when archiving fails", async () => {
    const api = fakeApi({
      archiveArticle: vi.fn(async () =>
        Promise.reject({ code: "storage", message: "Archivage impossible" }),
      ),
    });
    const { root } = await mounted(api);
    root.querySelector<HTMLElement>("[data-article-id]")!.click();
    await flush();

    root.querySelector<HTMLElement>('[data-action="archive-article"]')!.click();
    root.querySelector<HTMLElement>('[data-action="confirm-archive"]')!.click();
    await flush();

    expect(root.querySelector('[role="alert"]')?.textContent).toContain("Archivage impossible");
    expect(root.querySelector('[data-article-row-id="space::mars"]')).not.toBeNull();
    expect(root.querySelector(".reader-article")).not.toBeNull();
  });

  it("clears an article selected before automatic retention runs", async () => {
    const listArticles = vi
      .fn<InkRiverApi["listArticles"]>()
      .mockResolvedValueOnce([structuredClone(summary)])
      .mockResolvedValueOnce([]);
    const getArticle = vi
      .fn<InkRiverApi["getArticle"]>()
      .mockResolvedValueOnce(structuredClone(detail))
      .mockRejectedValueOnce({ code: "article_not_found", message: "Article archivé" });
    const api = fakeApi({
      listArticles,
      getArticle,
      refreshFeeds: vi.fn(async () => ({
        activeFeeds: 1,
        collectedArticles: 1,
        insertedArticles: 0,
        updatedArticles: 1,
        autoArchivedArticles: 1,
        extractedArticles: 2,
        extractionFailedArticles: 1,
        extractionSkippedArticles: 3,
        errors: [],
      })),
    });
    const { root } = await mounted(api);
    root.querySelector<HTMLElement>("[data-article-id]")!.click();
    await flush();

    root.querySelector<HTMLElement>('[data-action="refresh"]')!.click();
    await flush();

    expect(root.querySelector("[data-article-row-id]")).toBeNull();
    expect(root.querySelector(".reader-placeholder")).not.toBeNull();
    expect(root.querySelector('[role="status"]')?.textContent).toContain(
      "1 ancien article supprimé",
    );
    expect(root.querySelector('[role="status"]')?.textContent).not.toContain("extrait");
  });

  it("reports a partial refresh while retaining cached articles", async () => {
    const api = fakeApi({
      refreshFeeds: vi.fn(async () => ({
        activeFeeds: 2,
        collectedArticles: 1,
        insertedArticles: 1,
        updatedArticles: 0,
        autoArchivedArticles: 2,
        extractedArticles: 0,
        extractionFailedArticles: 1,
        extractionSkippedArticles: 2,
        errors: [{ feedId: "bread", feedUrl: "https://bread.example", stage: "HTTP request", message: "offline" }],
      })),
    });
    const { root } = await mounted(api);
    root.querySelector<HTMLElement>('[data-action="refresh"]')!.click();
    await flush();
    expect(root.querySelector('[role="alert"]')?.textContent).toContain("1 nouvel article");
    expect(root.querySelector('[role="alert"]')?.textContent).toContain(
      "2 anciens articles supprimés",
    );
    expect(root.querySelector('[role="alert"]')?.textContent).toContain("1 flux en erreur");
    expect(root.textContent).toContain("Observer Mars au crépuscule");
  });

  it("automatically hides a success notice after eight seconds", async () => {
    vi.useFakeTimers();
    try {
      const { root } = await mounted();
      root.querySelector<HTMLElement>('[data-action="refresh"]')!.click();
      await flushMicrotasks();

      expect(root.querySelector('[role="status"]')?.textContent).toBe("À jour");
      expect(root.querySelector(".notice-check svg")).not.toBeNull();
      expect(root.querySelector(".banner.notice")?.classList).toContain("is-entering");
      await vi.advanceTimersByTimeAsync(7_999);
      expect(root.querySelector('[role="status"]')).not.toBeNull();
      await vi.advanceTimersByTimeAsync(1);
      expect(root.querySelector(".banner.notice")?.classList).toContain("is-leaving");
      expect(root.querySelector('[role="status"]')).not.toBeNull();
      await vi.advanceTimersByTimeAsync(179);
      expect(root.querySelector('[role="status"]')).not.toBeNull();
      await vi.advanceTimersByTimeAsync(1);
      expect(root.querySelector('[role="status"]')).toBeNull();
    } finally {
      vi.clearAllTimers();
      vi.useRealTimers();
    }
  });

  it("pauses the notice timeout while it is hovered", async () => {
    vi.useFakeTimers();
    try {
      const { root } = await mounted();
      root.querySelector<HTMLElement>('[data-action="refresh"]')!.click();
      await flushMicrotasks();
      await vi.advanceTimersByTimeAsync(3_000);

      const banner = root.querySelector<HTMLElement>(".banner.notice")!;
      banner.dispatchEvent(new MouseEvent("mouseenter"));
      await vi.advanceTimersByTimeAsync(20_000);
      expect(root.querySelector('[role="status"]')).not.toBeNull();

      banner.dispatchEvent(new MouseEvent("mouseleave"));
      await vi.advanceTimersByTimeAsync(4_999);
      expect(root.querySelector('[role="status"]')).not.toBeNull();
      await vi.advanceTimersByTimeAsync(1);
      expect(root.querySelector(".banner.notice")?.classList).toContain("is-leaving");
      await vi.advanceTimersByTimeAsync(180);
      expect(root.querySelector('[role="status"]')).toBeNull();
    } finally {
      vi.clearAllTimers();
      vi.useRealTimers();
    }
  });

  it("pauses the notice timeout while its close button has focus", async () => {
    vi.useFakeTimers();
    try {
      const { root } = await mounted();
      root.querySelector<HTMLElement>('[data-action="refresh"]')!.click();
      await flushMicrotasks();

      const dismiss = root.querySelector<HTMLButtonElement>('[data-action="dismiss-notice"]')!;
      dismiss.focus();
      await vi.advanceTimersByTimeAsync(20_000);
      expect(root.querySelector('[role="status"]')).not.toBeNull();

      dismiss.blur();
      await vi.advanceTimersByTimeAsync(7_999);
      expect(root.querySelector('[role="status"]')).not.toBeNull();
      await vi.advanceTimersByTimeAsync(1);
      expect(root.querySelector(".banner.notice")?.classList).toContain("is-leaving");
      await vi.advanceTimersByTimeAsync(180);
      expect(root.querySelector('[role="status"]')).toBeNull();
    } finally {
      vi.clearAllTimers();
      vi.useRealTimers();
    }
  });

  it("keeps error banners visible", async () => {
    vi.useFakeTimers();
    try {
      const api = fakeApi({
        refreshFeeds: vi.fn(async () =>
          Promise.reject({ code: "refresh", message: "Actualisation impossible" }),
        ),
      });
      const { root } = await mounted(api);
      root.querySelector<HTMLElement>('[data-action="refresh"]')!.click();
      await flushMicrotasks();

      expect(root.querySelector('[role="alert"]')?.textContent).toContain(
        "Actualisation impossible",
      );
      await vi.advanceTimersByTimeAsync(60_000);
      expect(root.querySelector('[role="alert"]')?.textContent).toContain(
        "Actualisation impossible",
      );
    } finally {
      vi.clearAllTimers();
      vi.useRealTimers();
    }
  });

  it("renders subscription metadata and the persisted detailed error", async () => {
    const failedFeed: Feed = {
      ...structuredClone(feed),
      lastError: {
        stage: "HTTP request",
        message: "La connexion a expiré",
        occurredAt: "2026-08-12T17:45:00Z",
      },
    };
    const api = fakeApi({ listFeeds: vi.fn(async () => [failedFeed]) });
    const { root } = await mounted(api);

    root.querySelector<HTMLElement>('[data-action="subscriptions"]')!.click();

    const card = root.querySelector<HTMLElement>('[data-feed-card-id="stable-feed-id"]')!;
    expect(card.textContent).toContain("Carnet du ciel");
    expect(card.textContent).toContain("Claire du Ciel");
    expect(card.textContent).toContain("Une lettre pour observer le ciel");
    expect(card.textContent).toContain(feed.url);
    expect(card.textContent).toContain("Dernière publication");
    expect(card.textContent).toContain("Dernière actualisation réussie");
    expect(card.querySelector(".feed-error")?.textContent).toContain("HTTP request");
    expect(card.querySelector(".feed-error")?.textContent).toContain("La connexion a expiré");
  });

  it("reloads persisted feed errors after a partial refresh", async () => {
    const failedFeed: Feed = {
      ...structuredClone(feed),
      lastError: {
        stage: "feed parsing",
        message: "Document XML invalide",
        occurredAt: "2026-08-12T18:00:00Z",
      },
    };
    const listFeeds = vi
      .fn<InkRiverApi["listFeeds"]>()
      .mockResolvedValueOnce([structuredClone(feed)])
      .mockResolvedValueOnce([failedFeed]);
    const api = fakeApi({
      listFeeds,
      refreshFeeds: vi.fn(async () => ({
        activeFeeds: 1,
        collectedArticles: 0,
        insertedArticles: 0,
        updatedArticles: 0,
        autoArchivedArticles: 0,
        extractedArticles: 0,
        extractionFailedArticles: 0,
        extractionSkippedArticles: 0,
        errors: [{
          feedId: feed.id,
          feedUrl: feed.url,
          stage: "feed parsing",
          message: "Document XML invalide",
        }],
      })),
    });
    const { root } = await mounted(api);

    root.querySelector<HTMLElement>('[data-action="refresh"]')!.click();
    await flush();
    root.querySelector<HTMLElement>('[data-action="subscriptions"]')!.click();

    expect(listFeeds).toHaveBeenCalledTimes(2);
    expect(root.textContent).toContain("Consultez la page Abonnements");
    expect(root.querySelector(".feed-error")?.textContent).toContain("Document XML invalide");
  });

  it("adds, refreshes and deactivates subscriptions", async () => {
    const { root, api } = await mounted();
    root.querySelector<HTMLElement>('[data-action="subscriptions"]')!.click();
    expect(root.querySelector('[data-testid="feed-management"]')).not.toBeNull();
    root.querySelector<HTMLElement>('[data-action="add-subscription"]')!.click();
    expect(root.querySelector('[role="dialog"] [data-feed-card-id]')).toBeNull();
    const input = root.querySelector<HTMLInputElement>('input[name="url"]')!;
    input.value = "https://notes.medium.com/feed";
    input.dispatchEvent(new Event("input"));
    expect(root.querySelector<HTMLSelectElement>('select[name="platform"]')!.value).toBe("medium");
    root.querySelector<HTMLFormElement>("#feed-form")!.dispatchEvent(new Event("submit", { cancelable: true }));
    await flush();
    expect(api.addFeed).toHaveBeenCalledWith("https://notes.medium.com/feed", "medium");
    expect(api.refreshFeeds).not.toHaveBeenCalled();
    expect(api.refreshFeed).toHaveBeenCalledWith(feed.id);
    expect(root.querySelector('[role="status"]')?.textContent).toContain(
      "1 nouvel article",
    );
    root.querySelector<HTMLElement>('[data-action="toggle-feed"]')!.click();
    await flush();
    expect(api.setFeedActive).toHaveBeenCalledWith("stable-feed-id", false);
  });

  it("renders an accessible per-feed refresh button and disables it for inactive feeds", async () => {
    const inactiveFeed: Feed = {
      ...structuredClone(feed),
      id: "inactive-feed",
      url: "https://inactive.example/feed",
      isActive: false,
    };
    const api = fakeApi({
      listFeeds: vi.fn(async () => [structuredClone(feed), inactiveFeed]),
    });
    const { root } = await mounted(api);
    root.querySelector<HTMLElement>('[data-action="subscriptions"]')!.click();

    const activeButton = root.querySelector<HTMLButtonElement>(
      '[data-feed-card-id="stable-feed-id"] [data-action="refresh-feed"]',
    )!;
    const inactiveButton = root.querySelector<HTMLButtonElement>(
      '[data-feed-card-id="inactive-feed"] [data-action="refresh-feed"]',
    )!;
    expect(activeButton.title).toBe("Actualiser ce flux");
    expect(activeButton.getAttribute("aria-label")).toBe("Actualiser ce flux");
    expect(activeButton.textContent?.trim()).toBe("");
    expect(activeButton.disabled).toBe(false);
    expect(inactiveButton.disabled).toBe(true);
    expect(inactiveButton.title).toContain("Réactivez");
  });

  it("refreshes one feed, reloads the cache and remains on subscription management", async () => {
    const refreshedFeed: Feed = {
      ...structuredClone(feed),
      lastSuccessAt: "2026-08-22T12:00:00Z",
    };
    const listFeeds = vi
      .fn<InkRiverApi["listFeeds"]>()
      .mockResolvedValueOnce([structuredClone(feed)])
      .mockResolvedValueOnce([refreshedFeed]);
    const listArticles = vi
      .fn<InkRiverApi["listArticles"]>()
      .mockResolvedValueOnce([structuredClone(summary)])
      .mockResolvedValueOnce([structuredClone(summary), structuredClone(secondSummary)]);
    const api = fakeApi({ listFeeds, listArticles });
    const { root } = await mounted(api);
    root.querySelector<HTMLElement>('[data-action="subscriptions"]')!.click();

    root.querySelector<HTMLElement>('[data-action="refresh-feed"]')!.click();
    await flush();

    expect(api.refreshFeed).toHaveBeenCalledWith(feed.id);
    expect(api.refreshFeeds).not.toHaveBeenCalled();
    expect(listFeeds).toHaveBeenCalledTimes(2);
    expect(listArticles).toHaveBeenCalledTimes(2);
    expect(root.querySelector('[data-testid="feed-management"]')).not.toBeNull();
    expect(root.querySelector('[role="status"]')?.textContent).toContain(
      "1 nouvel article",
    );
    expect(root.querySelector('[role="status"]')?.textContent).not.toContain("Carnet du ciel");
    expect(root.querySelector(".notice-check")).toBeNull();
  });

  it("spins only the selected feed button and blocks every refresh while it runs", async () => {
    let finishRefresh!: (report: RefreshReport) => void;
    const refreshFeed = vi.fn<InkRiverApi["refreshFeed"]>(
      () => new Promise((resolve) => { finishRefresh = resolve; }),
    );
    const secondFeed: Feed = {
      ...structuredClone(feed),
      id: "second-feed",
      url: "https://second.example/feed",
    };
    const api = fakeApi({
      refreshFeed,
      listFeeds: vi.fn(async () => [structuredClone(feed), secondFeed]),
    });
    const { root } = await mounted(api);
    root.querySelector<HTMLElement>('[data-action="subscriptions"]')!.click();

    root.querySelector<HTMLElement>(
      '[data-feed-card-id="stable-feed-id"] [data-action="refresh-feed"]',
    )!.click();
    await flushMicrotasks();

    const buttons = Array.from(
      root.querySelectorAll<HTMLButtonElement>('[data-action="refresh-feed"]'),
    );
    expect(buttons.every((button) => button.disabled)).toBe(true);
    expect(buttons[0]?.getAttribute("aria-busy")).toBe("true");
    expect(buttons[1]?.getAttribute("aria-busy")).toBe("false");
    expect(root.querySelector<HTMLButtonElement>('[data-action="refresh"]')?.disabled).toBe(true);

    finishRefresh({
      activeFeeds: 1,
      collectedArticles: 0,
      insertedArticles: 0,
      updatedArticles: 0,
      autoArchivedArticles: 0,
      extractedArticles: 0,
      extractionFailedArticles: 0,
      extractionSkippedArticles: 0,
      errors: [],
    });
    await flush();
    expect(root.querySelector('[data-action="refresh-feed"]')?.getAttribute("aria-busy")).toBe("false");
  });

  it("preserves subscription scrolling throughout a targeted refresh", async () => {
    let finishRefresh!: (report: RefreshReport) => void;
    const api = fakeApi({
      refreshFeed: vi.fn(
        () => new Promise<RefreshReport>((resolve) => { finishRefresh = resolve; }),
      ),
    });
    const { root } = await mounted(api);
    root.querySelector<HTMLElement>('[data-action="subscriptions"]')!.click();
    root.querySelector<HTMLElement>(".feed-management")!.scrollTop = 640;

    root.querySelector<HTMLElement>('[data-action="refresh-feed"]')!.click();
    await flushMicrotasks();
    expect(root.querySelector<HTMLElement>(".feed-management")?.scrollTop).toBe(640);

    finishRefresh({
      activeFeeds: 1,
      collectedArticles: 1,
      insertedArticles: 0,
      updatedArticles: 1,
      autoArchivedArticles: 0,
      extractedArticles: 0,
      extractionFailedArticles: 0,
      extractionSkippedArticles: 0,
      errors: [],
    });
    await flush();
    expect(root.querySelector<HTMLElement>(".feed-management")?.scrollTop).toBe(640);
  });

  it("shows a dismissible detailed error toast and persists the error on the feed card", async () => {
    vi.useFakeTimers();
    try {
      const failedFeed: Feed = {
        ...structuredClone(feed),
        lastError: {
          stage: "HTTP request",
          message: "Connexion refusée",
          occurredAt: "2026-08-22T12:00:00Z",
        },
      };
      const listFeeds = vi
        .fn<InkRiverApi["listFeeds"]>()
        .mockResolvedValueOnce([structuredClone(feed)])
        .mockResolvedValueOnce([failedFeed]);
      const api = fakeApi({
        listFeeds,
        refreshFeed: vi.fn(async () => ({
          activeFeeds: 1,
          collectedArticles: 0,
          insertedArticles: 0,
          updatedArticles: 0,
          autoArchivedArticles: 0,
          extractedArticles: 0,
          extractionFailedArticles: 0,
          extractionSkippedArticles: 0,
          errors: [{
            feedId: feed.id,
            feedUrl: feed.url,
            stage: "HTTP request",
            message: "Connexion refusée",
          }],
        })),
      });
      const { root } = await mounted(api);
      root.querySelector<HTMLElement>('[data-action="subscriptions"]')!.click();
      root.querySelector<HTMLElement>('[data-action="refresh-feed"]')!.click();
      await flushMicrotasks();

      const toast = root.querySelector<HTMLElement>(".banner.error-notice");
      expect(toast?.querySelector('[role="alert"]')?.textContent).toContain(
        "HTTP request : Connexion refusée",
      );
      expect(root.querySelector(".feed-error")?.textContent).toContain("Connexion refusée");
      expect(root.querySelector('[data-testid="feed-management"]')).not.toBeNull();
      expect(root.querySelector('[data-action="dismiss-notice"]')).not.toBeNull();

      await vi.advanceTimersByTimeAsync(8_000);
      expect(root.querySelector(".banner.error-notice")?.classList).toContain("is-leaving");
      await vi.advanceTimersByTimeAsync(180);
      expect(root.querySelector(".banner.error-notice")).toBeNull();
    } finally {
      vi.clearAllTimers();
      vi.useRealTimers();
    }
  });

  it("cancels feed deletion without changing cached data", async () => {
    const api = fakeApi();
    const confirmer = vi.fn(() => false);
    const { root } = await mounted(api, vi.fn(async () => undefined), confirmer);
    root.querySelector<HTMLElement>('[data-action="subscriptions"]')!.click();

    root.querySelector<HTMLElement>('[data-action="delete-feed"]')!.click();
    await flush();

    expect(confirmer).toHaveBeenCalledWith(expect.stringContaining(feed.url));
    expect(confirmer).toHaveBeenCalledWith(expect.stringContaining("favoris"));
    expect(api.deleteFeed).not.toHaveBeenCalled();
    expect(root.textContent).toContain(feed.url);
    root.querySelector<HTMLElement>('[data-action="show-articles"]')!.click();
    expect(root.textContent).toContain(summary.title);
  });

  it("deletes a feed, reloads cached lists and closes its selected article", async () => {
    const listArticles = vi
      .fn<InkRiverApi["listArticles"]>()
      .mockResolvedValueOnce([structuredClone(summary)])
      .mockResolvedValueOnce([]);
    const listFeeds = vi
      .fn<InkRiverApi["listFeeds"]>()
      .mockResolvedValueOnce([structuredClone(feed)])
      .mockResolvedValueOnce([]);
    const api = fakeApi({ listArticles, listFeeds });
    const { root } = await mounted(api);
    root.querySelector<HTMLElement>("[data-article-id]")!.click();
    await flush();
    root.querySelector<HTMLElement>('[data-action="subscriptions"]')!.click();

    root.querySelector<HTMLElement>('[data-action="delete-feed"]')!.click();
    await flush();

    expect(api.deleteFeed).toHaveBeenCalledWith("stable-feed-id");
    expect(listFeeds).toHaveBeenCalledTimes(2);
    expect(listArticles).toHaveBeenCalledTimes(2);
    expect(root.textContent).toContain("Abonnement supprimé avec 1 article supprimé.");
    expect(root.textContent).toContain("Aucun abonnement");
    root.querySelector<HTMLElement>('[data-action="show-articles"]')!.click();
    expect(root.textContent).toContain("Sélectionnez un article");
    expect(root.textContent).not.toContain(summary.title);
  });

  it("keeps the feed and cached articles visible when deletion fails", async () => {
    const api = fakeApi({
      deleteFeed: vi.fn(async () => Promise.reject({ code: "storage", message: "Suppression impossible" })),
    });
    const { root } = await mounted(api);
    root.querySelector<HTMLElement>('[data-action="subscriptions"]')!.click();

    root.querySelector<HTMLElement>('[data-action="delete-feed"]')!.click();
    await flush();

    expect(root.querySelector('[role="alert"]')?.textContent).toContain("Suppression impossible");
    expect(root.textContent).toContain(feed.url);
    root.querySelector<HTMLElement>('[data-action="show-articles"]')!.click();
    expect(root.textContent).toContain(summary.title);
  });

  it("shows the source and original actions for an excerpt", async () => {
    const { root, opener } = await mounted();
    root.querySelector<HTMLElement>("[data-article-id]")!.click();
    await flush();

    const sourceLink = root.querySelector<HTMLElement>('[data-action="open-source"]')!;
    expect(sourceLink.textContent).toContain("space.example");
    expect(sourceLink.getAttribute("title")).toBe("https://space.example/mars");
    sourceLink.click();
    await flush();
    root.querySelector<HTMLElement>('[data-action="open-original"]')!.click();
    await flush();

    expect(opener).toHaveBeenNthCalledWith(1, "https://space.example/mars");
    expect(opener).toHaveBeenNthCalledWith(2, "https://space.example/mars");
  });

  it("shows a source action but no original button for full content", async () => {
    const fullDetail = { ...detail, contentKind: "full" as const };
    const api = fakeApi({ getArticle: vi.fn(async () => structuredClone(fullDetail)) });
    const { root, opener } = await mounted(api);
    root.querySelector<HTMLElement>("[data-article-id]")!.click();
    await flush();

    expect(root.querySelector('[data-action="open-original"]')).toBeNull();
    root.querySelector<HTMLElement>('[data-action="open-source"]')!.click();
    await flush();
    expect(opener).toHaveBeenCalledWith("https://space.example/mars");
    expect(root.querySelector(".reader-article")).not.toBeNull();
  });

  it("renders unavailable source states without an opener action", async () => {
    const missingApi = fakeApi({
      getArticle: vi.fn(async () => ({ ...structuredClone(detail), url: null })),
    });
    const missing = await mounted(missingApi);
    missing.root.querySelector<HTMLElement>("[data-article-id]")!.click();
    await flush();
    expect(missing.root.querySelector(".article-source")?.textContent).toContain("Source indisponible");
    expect(missing.root.querySelector('.article-source [data-action="open-source"]')).toBeNull();
    expect(
      missing.root.querySelector<HTMLButtonElement>(
        '.reader-footer [data-action="open-source"]',
      )?.disabled,
    ).toBe(true);
    expect(missing.root.querySelector('[data-action="open-original"]')).toBeNull();
    expect(missing.opener).not.toHaveBeenCalled();

    const invalidApi = fakeApi({
      getArticle: vi.fn(async () => ({ ...structuredClone(detail), url: "mailto:author@example.com" })),
    });
    const invalid = await mounted(invalidApi);
    invalid.root.querySelector<HTMLElement>("[data-article-id]")!.click();
    await flush();
    expect(invalid.root.querySelector(".article-source")?.textContent).toContain("Source non prise en charge");
    expect(invalid.root.querySelector('.article-source [data-action="open-source"]')).toBeNull();
    expect(
      invalid.root.querySelector<HTMLButtonElement>(
        '.reader-footer [data-action="open-source"]',
      )?.disabled,
    ).toBe(true);
    expect(invalid.root.querySelector('[data-action="open-original"]')).toBeNull();
    expect(invalid.opener).not.toHaveBeenCalled();
  });

  it("keeps the article visible when opening its source fails", async () => {
    const opener = vi.fn(async () => Promise.reject(new Error("Navigateur indisponible")));
    const { root } = await mounted(undefined, opener);
    root.querySelector<HTMLElement>("[data-article-id]")!.click();
    await flush();

    root.querySelector<HTMLElement>('[data-action="open-source"]')!.click();
    await flush();
    expect(root.querySelector('[role="alert"]')?.textContent).toContain("Navigateur indisponible");
    expect(root.querySelector(".reader-article")?.textContent).toContain("Observer Mars au crépuscule");
  });

  it("opens relative article links in the system browser without navigating the frame", async () => {
    const linkedDetail = {
      ...detail,
      content: '<p><a href="/members/story"><span>Suite réservée</span></a></p>',
    };
    const api = fakeApi({ getArticle: vi.fn(async () => structuredClone(linkedDetail)) });
    const { root, opener } = await mounted(api);
    root.querySelector<HTMLElement>("[data-article-id]")!.click();
    await flush();

    const frame = root.querySelector<HTMLIFrameElement>(".article-content")!;
    expect(frame.getAttribute("sandbox")).toBe("allow-scripts");
    expect(frame.srcdoc).toContain("Content-Security-Policy");
    expect(frame.srcdoc).toContain("data-external-href=\"/members/story\"");
    window.dispatchEvent(
      new MessageEvent("message", {
        data: { type: "inkriver:article-link", href: "/members/story" },
        source: frame.contentWindow,
      }),
    );
    await flush();

    expect(opener).toHaveBeenCalledWith("https://space.example/members/story");
    expect(root.querySelector(".article-content")).not.toBeNull();
  });

  it("resizes the article frame so the reader owns the complete scroll", async () => {
    const { root } = await mounted();
    root.querySelector<HTMLElement>("[data-article-id]")!.click();
    await flush();

    const frame = root.querySelector<HTMLIFrameElement>(".article-content")!;
    expect(frame.getAttribute("scrolling")).toBe("no");
    window.dispatchEvent(
      new MessageEvent("message", {
        data: { type: "inkriver:article-height", height: 1234.2 },
        source: frame.contentWindow,
      }),
    );
    expect(frame.style.height).toBe("1235px");

    window.dispatchEvent(
      new MessageEvent("message", {
        data: { type: "inkriver:article-height", height: -1 },
        source: frame.contentWindow,
      }),
    );
    expect(frame.style.height).toBe("1235px");
  });

  it("shows the Top button after one viewport and hides it below three quarters", async () => {
    const { root } = await mounted();
    expect(root.querySelector('[data-action="reader-top"]')).toBeNull();
    root.querySelector<HTMLElement>("[data-article-id]")!.click();
    await flush();

    const reader = root.querySelector<HTMLElement>(".reader")!;
    const button = root.querySelector<HTMLButtonElement>('[data-action="reader-top"]')!;
    Object.defineProperty(reader, "clientHeight", { configurable: true, value: 600 });

    expect(button.getAttribute("title")).toBe("Revenir en haut");
    expect(button.getAttribute("aria-label")).toBe("Revenir en haut");
    expect(button.getAttribute("aria-hidden")).toBe("true");
    expect(button.tabIndex).toBe(-1);
    expect(button.querySelector("svg")).not.toBeNull();

    reader.scrollTop = 600;
    reader.dispatchEvent(new Event("scroll"));
    expect(button.classList).not.toContain("visible");

    reader.scrollTop = 601;
    reader.dispatchEvent(new Event("scroll"));
    expect(button.classList).toContain("visible");
    expect(button.getAttribute("aria-hidden")).toBe("false");
    expect(button.tabIndex).toBe(0);

    reader.scrollTop = 451;
    reader.dispatchEvent(new Event("scroll"));
    expect(button.classList).toContain("visible");

    reader.scrollTop = 450;
    reader.dispatchEvent(new Event("scroll"));
    expect(button.classList).not.toContain("visible");
    expect(button.getAttribute("aria-hidden")).toBe("true");
    expect(button.tabIndex).toBe(-1);
  });

  it("shows mobile reading position and remaining length in a vertical indicator", async () => {
    const restoreViewport = installMobileViewport();
    try {
      const { root } = await mounted();
      expect(root.querySelector(".reader-progress")).toBeNull();
      root.querySelector<HTMLElement>("[data-article-id]")!.click();
      await flush();

      const reader = root.querySelector<HTMLElement>(".reader")!;
      const track = root.querySelector<HTMLElement>(".reader-progress")!;
      const thumb = track.querySelector<HTMLElement>(".reader-progress-thumb")!;
      Object.defineProperty(reader, "clientHeight", { configurable: true, value: 600 });
      Object.defineProperty(reader, "scrollHeight", { configurable: true, value: 2400 });
      Object.defineProperty(track, "clientHeight", { configurable: true, value: 480 });

      reader.scrollTop = 0;
      reader.dispatchEvent(new Event("scroll"));
      expect(track.getAttribute("aria-hidden")).toBe("true");
      expect(track.classList).toContain("visible");
      expect(thumb.style.height).toBe("120px");
      expect(thumb.style.top).toBe("0px");

      reader.scrollTop = 900;
      reader.dispatchEvent(new Event("scroll"));
      expect(thumb.style.top).toBe("180px");

      reader.scrollTop = 1800;
      reader.dispatchEvent(new Event("scroll"));
      expect(thumb.style.top).toBe("360px");

      Object.defineProperty(reader, "scrollHeight", { configurable: true, value: 600 });
      reader.scrollTop = 0;
      reader.dispatchEvent(new Event("scroll"));
      expect(track.classList).not.toContain("visible");
      expect(thumb.style.height).toBe("");
      expect(thumb.style.top).toBe("");
    } finally {
      restoreViewport();
    }
  });

  it("scrolls the reader smoothly to the top and respects reduced motion", async () => {
    const { root } = await mounted();
    root.querySelector<HTMLElement>("[data-article-id]")!.click();
    await flush();

    const reader = root.querySelector<HTMLElement>(".reader")!;
    const button = root.querySelector<HTMLButtonElement>('[data-action="reader-top"]')!;
    const scrollTo = vi.fn();
    Object.defineProperty(reader, "scrollTo", { configurable: true, value: scrollTo });

    button.click();
    expect(scrollTo).toHaveBeenLastCalledWith({ top: 0, behavior: "smooth" });

    const originalMatchMedia = window.matchMedia;
    Object.defineProperty(window, "matchMedia", {
      configurable: true,
      value: vi.fn((media: string) => ({ matches: true, media }) as MediaQueryList),
    });
    try {
      button.click();
      expect(scrollTo).toHaveBeenLastCalledWith({ top: 0, behavior: "auto" });
    } finally {
      Object.defineProperty(window, "matchMedia", {
        configurable: true,
        value: originalMatchMedia,
      });
    }
  });

  it("starts with the Top button hidden when another article is opened", async () => {
    const api = fakeApi({
      listArticles: vi.fn(async () => [structuredClone(summary), structuredClone(secondSummary)]),
      getArticle: vi.fn(async (articleId) =>
        structuredClone(articleId === secondDetail.id ? secondDetail : detail),
      ),
    });
    const { root } = await mounted(api);
    root.querySelector<HTMLElement>('[data-article-id="space::mars"]')!.click();
    await flush();

    const reader = root.querySelector<HTMLElement>(".reader")!;
    Object.defineProperty(reader, "clientHeight", { configurable: true, value: 600 });
    reader.scrollTop = 700;
    reader.dispatchEvent(new Event("scroll"));
    expect(root.querySelector(".reader-top-button")?.classList).toContain("visible");

    root.querySelector<HTMLElement>('[data-article-id="space::venus"]')!.click();
    await flush();
    expect(root.querySelector(".reader-top-button")?.classList).not.toContain("visible");
    expect(root.querySelector<HTMLElement>(".reader")?.scrollTop).toBe(0);
  });

  it("keeps fragment links inside the article frame", async () => {
    const linkedDetail = {
      ...detail,
      content: '<a href="#notes">Notes</a><h2 id="notes">Notes</h2>',
    };
    const api = fakeApi({ getArticle: vi.fn(async () => structuredClone(linkedDetail)) });
    const { root, opener } = await mounted(api);
    root.querySelector<HTMLElement>("[data-article-id]")!.click();
    await flush();

    const frame = root.querySelector<HTMLIFrameElement>(".article-content")!;
    expect(frame.srcdoc).toContain('data-internal-fragment="notes"');
    expect(frame.srcdoc).toContain("destination.scrollIntoView");
    expect(opener).not.toHaveBeenCalled();
  });

  it("opens article images in a reader lightbox instead of the system browser", async () => {
    const imageDetail = {
      ...structuredClone(detail),
      content: '<a href="https://photos.example/full"><img src="/mars.jpg" alt="Mars au crépuscule"></a>',
    };
    const api = fakeApi({ getArticle: vi.fn(async () => imageDetail) });
    const { root, opener } = await mounted(api);
    root.querySelector<HTMLElement>("[data-article-id]")!.click();
    await flush();

    let frame = root.querySelector<HTMLIFrameElement>(".article-content")!;
    window.dispatchEvent(
      new MessageEvent("message", {
        data: {
          type: "inkriver:article-image",
          src: "/mars.jpg",
          alt: "Mars au crépuscule",
          imageId: "0",
        },
        source: frame.contentWindow,
      }),
    );

    const lightbox = root.querySelector<HTMLElement>(".image-lightbox")!;
    const dialog = lightbox.querySelector<HTMLElement>('[role="dialog"]')!;
    const image = lightbox.querySelector<HTMLImageElement>(".image-lightbox-image")!;
    const close = lightbox.querySelector<HTMLButtonElement>(
      '[data-action="close-image-zoom"]',
    )!;
    expect(lightbox.classList).toContain("is-entering");
    expect(dialog.getAttribute("aria-label")).toBe("Image agrandie : Mars au crépuscule");
    expect(image.getAttribute("src")).toBe("https://space.example/mars.jpg");
    expect(image.getAttribute("alt")).toBe("Mars au crépuscule");
    expect(close.getAttribute("title")).toBe("Fermer l’image");
    expect(document.activeElement).toBe(close);
    expect(opener).not.toHaveBeenCalled();

    vi.useFakeTimers();
    try {
      close.click();
      expect(root.querySelector(".image-lightbox")?.classList).toContain("is-leaving");
      await vi.advanceTimersByTimeAsync(180);
      expect(root.querySelector(".image-lightbox")).toBeNull();

      frame = root.querySelector<HTMLIFrameElement>(".article-content")!;
      window.dispatchEvent(
        new MessageEvent("message", {
          data: { type: "inkriver:article-image", src: "/mars.jpg", imageId: "0" },
          source: frame.contentWindow,
        }),
      );
      root.querySelector<HTMLElement>(".image-lightbox")!.click();
      await vi.advanceTimersByTimeAsync(180);
      expect(root.querySelector(".image-lightbox")).toBeNull();

      frame = root.querySelector<HTMLIFrameElement>(".article-content")!;
      window.dispatchEvent(
        new MessageEvent("message", {
          data: { type: "inkriver:article-image", src: "/mars.jpg", imageId: "0" },
          source: frame.contentWindow,
        }),
      );
      root.querySelector<HTMLElement>(".image-lightbox")!.dispatchEvent(
        new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
      );
      await vi.advanceTimersByTimeAsync(180);
      expect(root.querySelector(".image-lightbox")).toBeNull();
    } finally {
      vi.clearAllTimers();
      vi.useRealTimers();
    }
  });

  it("ignores unsupported image sources and closes the zoom when changing article", async () => {
    const api = fakeApi({
      listArticles: vi.fn(async () => [structuredClone(summary), structuredClone(secondSummary)]),
      getArticle: vi.fn(async (articleId) =>
        structuredClone(articleId === secondDetail.id ? secondDetail : detail),
      ),
    });
    const { root } = await mounted(api);
    root.querySelector<HTMLElement>('[data-article-id="space::mars"]')!.click();
    await flush();

    let frame = root.querySelector<HTMLIFrameElement>(".article-content")!;
    window.dispatchEvent(
      new MessageEvent("message", {
        data: { type: "inkriver:article-image", src: "http://unsafe.example/mars.jpg", imageId: "0" },
        source: frame.contentWindow,
      }),
    );
    expect(root.querySelector(".image-lightbox")).toBeNull();

    window.dispatchEvent(
      new MessageEvent("message", {
        data: { type: "inkriver:article-image", src: "https://safe.example/mars.jpg", imageId: "0" },
        source: frame.contentWindow,
      }),
    );
    expect(root.querySelector(".image-lightbox")).not.toBeNull();

    root.querySelector<HTMLElement>('[data-article-id="space::venus"]')!.click();
    await flush();
    expect(root.querySelector(".image-lightbox")).toBeNull();
    expect(root.querySelector(".reader-article")?.textContent).toContain("Observer Vénus");
  });

  it("rejects non-HTTP article links and reports opener failures", async () => {
    const linkedDetail = {
      ...detail,
      content: '<a href="mailto:test@example.com">Mail</a><a href="https://example.com/private">Privé</a>',
    };
    const api = fakeApi({ getArticle: vi.fn(async () => structuredClone(linkedDetail)) });
    const opener = vi.fn(async () => Promise.reject(new Error("Navigateur indisponible")));
    const mountedApp = await mounted(api, opener);
    mountedApp.root.querySelector<HTMLElement>("[data-article-id]")!.click();
    await flush();

    let frame = mountedApp.root.querySelector<HTMLIFrameElement>(".article-content")!;
    window.dispatchEvent(
      new MessageEvent("message", {
        data: { type: "inkriver:article-link", href: "mailto:test@example.com" },
        source: frame.contentWindow,
      }),
    );
    await flush();
    expect(opener).not.toHaveBeenCalled();
    expect(mountedApp.root.querySelector('[role="alert"]')?.textContent).toContain(
      "Seuls les liens HTTP(S)",
    );

    frame = mountedApp.root.querySelector<HTMLIFrameElement>(".article-content")!;
    window.dispatchEvent(
      new MessageEvent("message", {
        data: { type: "inkriver:article-link", href: "https://example.com/private" },
        source: frame.contentWindow,
      }),
    );
    await flush();
    expect(opener).toHaveBeenCalledWith("https://example.com/private");
    expect(mountedApp.root.querySelector('[role="alert"]')?.textContent).toContain(
      "Navigateur indisponible",
    );
  });
  it("configures a first synchronization group from subscription management", async () => {
    const api = fakeApi();
    const { root } = await mounted(api);
    root.querySelector<HTMLElement>('[data-action="subscriptions"]')!.click();
    root.querySelector<HTMLElement>('[data-action="open-sync"]')!.click();
    await flush();

    expect(root.querySelector("#configure-sync-form")).not.toBeNull();
    const form = root.querySelector<HTMLFormElement>("#configure-sync-form")!;
    form.querySelector<HTMLInputElement>('[name="webdavBaseUrl"]')!.value =
      "https://cloud.example/dav/inkriver";
    form.querySelector<HTMLInputElement>('[name="webdavUsername"]')!.value = "alice";
    form.querySelector<HTMLInputElement>('[name="webdavPassword"]')!.value = "secret";
    form.querySelector<HTMLInputElement>('[name="deviceName"]')!.value = "Linux";
    form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
    await flush();

    expect(api.configureSyncGroup).toHaveBeenCalledWith(
      "https://cloud.example/dav/inkriver",
      "alice",
      "secret",
      "Linux",
    );
    expect(root.textContent).toContain("Serveur WebDAV");
    expect(root.querySelector<HTMLInputElement>('[data-sync-device-form] input')?.value).toBe(
      "Linux",
    );
  });

  it("scans an invitation on mobile and joins without putting the WebDAV password in the QR", async () => {
    const scanner = vi.fn(async () => "inkriver://pair/scanned-invitation");
    const api = fakeApi();
    const { root } = await mounted(api, undefined, undefined, scanner);
    root.querySelector<HTMLElement>('[data-action="subscriptions"]')!.click();
    root.querySelector<HTMLElement>('[data-action="open-sync"]')!.click();
    await flush();

    root.querySelector<HTMLElement>('[data-action="scan-pairing-code"]')!.click();
    await flush();
    const form = root.querySelector<HTMLFormElement>("#join-sync-form")!;
    expect(form.querySelector<HTMLTextAreaElement>('[name="invitation"]')!.value).toBe(
      "inkriver://pair/scanned-invitation",
    );
    form.querySelector<HTMLInputElement>('[name="webdavPassword"]')!.value = "separate-secret";
    form.querySelector<HTMLInputElement>('[name="deviceName"]')!.value = "Android";
    form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
    await flush();

    expect(scanner).toHaveBeenCalledOnce();
    expect(api.joinSyncGroup).toHaveBeenCalledWith(
      "inkriver://pair/scanned-invitation",
      "separate-secret",
      "Android",
    );
    expect(root.querySelector<HTMLInputElement>('[data-sync-device-form] input')?.value).toBe(
      "Android",
    );
  });

  it("displays a confidential pairing QR and manages synchronized devices", async () => {
    const configured = {
      ...emptySyncRuntime,
      configured: true,
      webdavBaseUrl: "https://cloud.example/dav/inkriver",
      webdavUsername: "alice",
      keyId: "key-123",
      devices: [
        { deviceId: "linux", displayName: "Linux", isLocal: true, revokedAt: null },
        { deviceId: "phone", displayName: "Téléphone", isLocal: false, revokedAt: null },
      ],
    };
    const api = fakeApi({ syncPairingStatus: vi.fn(async () => configured) });
    const confirmer = vi.fn(() => true);
    const { root } = await mounted(api, undefined, confirmer);
    root.querySelector<HTMLElement>('[data-action="subscriptions"]')!.click();
    root.querySelector<HTMLElement>('[data-action="open-sync"]')!.click();
    await flush();

    root.querySelector<HTMLElement>('[data-action="create-pairing-invitation"]')!.click();
    await flush();
    expect(api.pairingInvitation).toHaveBeenCalledOnce();
    expect(root.querySelector<HTMLImageElement>('.pairing-invitation img')?.src).toContain(
      "data:image/svg+xml;base64,",
    );
    expect(root.textContent).toContain("contient la clé de chiffrement");
    expect(root.textContent).not.toContain("separate-secret");

    root.querySelector<HTMLElement>('[data-action="revoke-sync-device"]')!.click();
    await flush();
    expect(confirmer).toHaveBeenCalledWith(expect.stringContaining("Téléphone"));
    expect(api.revokeSyncDevice).toHaveBeenCalledWith("phone");
  });

  it("synchronizes manually, reloads projections and keeps subscription management open", async () => {
    const configured = {
      ...emptySyncRuntime,
      configured: true,
      webdavBaseUrl: "https://cloud.example/dav/inkriver",
      webdavUsername: "alice",
      keyId: "key-123",
      devices: [],
    };
    const synchronized = {
      ...configured,
      lastAttemptAt: "2026-08-28T12:30:00Z",
      lastSuccessAt: "2026-08-28T12:30:00Z",
      lastReport: {
        uploadedSegments: 1,
        reusedSegments: 0,
        exportedEvents: 2,
        downloadedSegments: 1,
        receivedEvents: 3,
        importedEvents: 3,
        duplicateEvents: 0,
        appliedEvents: 3,
        pendingEvents: 0,
      },
    };
    const syncPairingStatus = vi
      .fn<InkRiverApi["syncPairingStatus"]>()
      .mockResolvedValueOnce(configured)
      .mockResolvedValue(synchronized);
    const api = fakeApi({
      syncPairingStatus,
      synchronizeNow: vi.fn(async () => ({
        uploadedSegments: 1,
        reusedSegments: 0,
        exportedEvents: 2,
        downloadedSegments: 1,
        receivedEvents: 3,
        importedEvents: 3,
        duplicateEvents: 0,
        appliedEvents: 3,
        pendingEvents: 0,
      })),
    });
    const { root } = await mounted(api);
    root.querySelector<HTMLElement>('[data-action="subscriptions"]')!.click();
    root.querySelector<HTMLElement>('[data-action="open-sync"]')!.click();
    await flush();

    root.querySelector<HTMLElement>('[data-action="synchronize-now"]')!.click();
    await flush();
    expect(api.synchronizeNow).toHaveBeenCalledOnce();
    expect(api.listArticles).toHaveBeenCalledTimes(2);
    expect(api.listFeeds).toHaveBeenCalledTimes(2);
    expect(root.querySelector('[data-testid="feed-management"]')).not.toBeNull();
    expect(root.querySelector(".sync-dialog")).not.toBeNull();
    expect(root.textContent).toContain("2 changements envoyés, 3 appliqués");
    expect(root.textContent).toContain("Dernière synchronisation réussie");
  });

  it("keeps cached content and shows the detailed error when manual synchronization fails", async () => {
    const configured = {
      ...emptySyncRuntime,
      configured: true,
      webdavBaseUrl: "https://cloud.example/dav/inkriver",
      webdavUsername: "alice",
      keyId: "key-123",
      devices: [],
    };
    const failed = {
      ...configured,
      lastAttemptAt: "2026-08-28T12:35:00Z",
      lastError: {
        stage: "Transport WebDAV",
        message: "PROPFIND returned HTTP status 503",
        occurredAt: "2026-08-28T12:35:00Z",
      },
    };
    const syncPairingStatus = vi
      .fn<InkRiverApi["syncPairingStatus"]>()
      .mockResolvedValueOnce(configured)
      .mockResolvedValue(failed);
    const api = fakeApi({
      syncPairingStatus,
      synchronizeNow: vi.fn(async () => {
        throw { code: "sync_failed", message: "PROPFIND returned HTTP status 503" };
      }),
    });
    const { root } = await mounted(api);
    root.querySelector<HTMLElement>('[data-action="subscriptions"]')!.click();
    root.querySelector<HTMLElement>('[data-action="open-sync"]')!.click();
    await flush();

    root.querySelector<HTMLElement>('[data-action="synchronize-now"]')!.click();
    await flush();
    expect(root.querySelector(".sync-persisted-error")?.textContent).toContain(
      "PROPFIND returned HTTP status 503",
    );
    expect(root.textContent).toContain("Carnet du ciel");
    expect(root.querySelector('[data-testid="feed-management"]')).not.toBeNull();
    expect(root.textContent).toContain("Dernière erreur · Transport WebDAV");
  });

  it("identifies a persisted synchronization with pending events as partial", async () => {
    const api = fakeApi({
      syncPairingStatus: vi.fn(async () => ({
        ...emptySyncRuntime,
        configured: true,
        webdavBaseUrl: "https://cloud.example/dav/inkriver",
        webdavUsername: "alice",
        keyId: "key-123",
        devices: [],
        lastAttemptAt: "2026-08-28T12:40:00Z",
        lastSuccessAt: "2026-08-28T12:40:00Z",
        lastReport: {
          uploadedSegments: 1,
          reusedSegments: 0,
          exportedEvents: 1,
          downloadedSegments: 1,
          receivedEvents: 2,
          importedEvents: 2,
          duplicateEvents: 0,
          appliedEvents: 1,
          pendingEvents: 1,
        },
      })),
    });
    const { root } = await mounted(api);
    root.querySelector<HTMLElement>('[data-action="subscriptions"]')!.click();
    root.querySelector<HTMLElement>('[data-action="open-sync"]')!.click();
    await flush();

    expect(root.textContent).toContain("Dernière synchronisation partielle");
    expect(root.textContent).toContain("1 en attente");
  });

  it("deletes only the local synchronization configuration after confirmation", async () => {
    const configured = {
      ...emptySyncRuntime,
      configured: true,
      webdavBaseUrl: "https://cloud.example/dav/inkriver",
      webdavUsername: "alice",
      keyId: "key-123",
      devices: [],
    };
    const api = fakeApi({ syncPairingStatus: vi.fn(async () => configured) });
    const confirmer = vi.fn(() => true);
    const { root } = await mounted(api, undefined, confirmer);
    root.querySelector<HTMLElement>('[data-action="subscriptions"]')!.click();
    root.querySelector<HTMLElement>('[data-action="open-sync"]')!.click();
    await flush();

    root.querySelector<HTMLElement>('[data-action="delete-sync-configuration"]')!.click();
    await flush();
    expect(confirmer).toHaveBeenCalledWith(expect.stringContaining("fichiers WebDAV distants"));
    expect(api.deleteSyncConfiguration).toHaveBeenCalledOnce();
    expect(root.querySelector("#configure-sync-form")).not.toBeNull();
    expect(root.textContent).toContain("Carnet du ciel");
    expect(root.textContent).toContain("Configuration de synchronisation supprimée");
  });
});

describe("view helpers", () => {
  it("detects known platforms without accepting lookalike domains", () => {
    expect(detectPlatform("https://medium.com/feed/@inkriver")).toBe("medium");
    expect(detectPlatform("https://letters.substack.com/feed")).toBe("substack");
    expect(detectPlatform("https://substack.com.example/feed")).toBe("other");
  });

  it("shows original links only for incomplete content", () => {
    expect(canOpenOriginal(detail)).toBe(true);
    expect(canOpenOriginal({ ...detail, contentKind: "missing" })).toBe(true);
    expect(canOpenOriginal({ ...detail, contentKind: "unknown" })).toBe(true);
    expect(canOpenOriginal({ ...detail, contentKind: "full" })).toBe(false);
    expect(canOpenOriginal({ ...detail, contentKind: "extracted" })).toBe(false);
    expect(canOpenOriginal({ ...detail, url: null })).toBe(false);
    expect(canOpenOriginal({ ...detail, url: "mailto:author@example.com" })).toBe(false);
  });

  it("extracts the host only from absolute HTTP(S) article sources", () => {
    expect(articleSourceHost("https://space.example:8443/mars")).toBe("space.example:8443");
    expect(articleSourceHost("mailto:author@example.com")).toBeNull();
    expect(articleSourceHost("not a URL")).toBeNull();
    expect(articleSourceHost(null)).toBeNull();
  });

  it("extracts structured errors and falls back to strings", () => {
    expect(errorMessage({ code: "storage", message: "Base indisponible" })).toBe("Base indisponible");
    expect(errorMessage("offline")).toBe("offline");
  });

  it("resolves only HTTP(S) article links", () => {
    expect(resolveExternalArticleUrl("/members/story", detail.url)).toBe(
      "https://space.example/members/story",
    );
    expect(resolveExternalArticleUrl("https://example.com/read")).toBe(
      "https://example.com/read",
    );
    expect(() => resolveExternalArticleUrl("mailto:test@example.com", detail.url)).toThrow(
      "Seuls les liens HTTP(S)",
    );
    expect(() => resolveExternalArticleUrl("relative-without-base")).toThrow("Lien invalide");
  });

  it("resolves only safe article image sources", () => {
    expect(resolveArticleImageUrl("/images/mars.jpg", detail.url)).toBe(
      "https://space.example/images/mars.jpg",
    );
    expect(resolveArticleImageUrl("data:image/png;base64,AAAA")).toBe(
      "data:image/png;base64,AAAA",
    );
    expect(() => resolveArticleImageUrl("http://space.example/mars.jpg")).toThrow(
      "HTTPS ou data:",
    );
    expect(() =>
      resolveArticleImageUrl("https://user:secret@space.example/mars.jpg"),
    ).toThrow("HTTPS ou data:");
    expect(() => resolveArticleImageUrl("/mars.jpg")).toThrow("Image invalide");
  });

  it("reads and writes text-size preferences with a safe fallback", () => {
    const values = new Map<string, string>();
    const storage = {
      getItem: vi.fn((key: string) => values.get(key) ?? null),
      setItem: vi.fn((key: string, value: string) => {
        values.set(key, value);
      }),
    };
    expect(readArticleTextSize(storage)).toBe("medium");
    writeArticleTextSize(storage, "large");
    expect(readArticleTextSize(storage)).toBe("large");

    values.set("inkriver.articleTextSize", "enormous");
    expect(readArticleTextSize(storage)).toBe("medium");
    expect(readArticleTextSize({ getItem: () => { throw new Error("denied"); } })).toBe(
      "medium",
    );
    expect(() =>
      writeArticleTextSize({ setItem: () => { throw new Error("denied"); } }, "small"),
    ).not.toThrow();
  });

  it("neutralizes external frame navigation while preserving fragment links", () => {
    const content = prepareArticleContent(
      '<a href="/story" target="_blank">Story</a><a href="#notes" target="_top">Notes</a>',
    );
    const document = new DOMParser().parseFromString(content, "text/html");
    const links = document.querySelectorAll("a");
    expect(links[0]?.getAttribute("href")).toBe("about:srcdoc#");
    expect((links[0] as HTMLElement | undefined)?.dataset.externalHref).toBe("/story");
    expect(links[0]?.hasAttribute("target")).toBe(false);
    expect(links[1]?.getAttribute("href")).toBe("about:srcdoc#");
    expect((links[1] as HTMLElement | undefined)?.dataset.externalHref).toBeUndefined();
    expect((links[1] as HTMLElement | undefined)?.dataset.internalFragment).toBe("notes");
    expect(links[1]?.hasAttribute("target")).toBe(false);
  });

  it("makes article images keyboard-accessible zoom controls", () => {
    const content = prepareArticleContent(
      '<a href="https://photos.example/full"><img src="/mars.jpg" alt="Mars"></a>',
    );
    const document = new DOMParser().parseFromString(content, "text/html");
    const image = document.querySelector<HTMLImageElement>("img")!;
    expect(image.dataset.zoomableImage).toBe("0");
    expect(image.tabIndex).toBe(0);
    expect(image.getAttribute("role")).toBe("button");
    expect(image.getAttribute("aria-label")).toBe("Agrandir l’image : Mars");
    expect(document.querySelector("a")?.dataset.externalHref).toBe(
      "https://photos.example/full",
    );
  });

  it("builds a hash-protected iframe bridge for links and text sizing", () => {
    const document = buildArticleDocument(
      '<a href="https://example.com/read">Read</a>',
      "small",
    );
    const computedHash = `sha256-${createHash("sha256")
      .update(ARTICLE_BRIDGE_SCRIPT, "utf8")
      .digest("base64")}`;
    expect(computedHash).toBe(ARTICLE_BRIDGE_CSP_HASH);
    expect(document).toContain(`script-src '${ARTICLE_BRIDGE_CSP_HASH}'`);
    expect(document).toContain(`<script>${ARTICLE_BRIDGE_SCRIPT}</script>`);
    expect(document).not.toContain("nonce=");
    expect(tauriConfig.app.security.csp).toContain(
      `script-src 'self' '${ARTICLE_BRIDGE_CSP_HASH}'`,
    );
    expect(document).toContain('window.parent.postMessage({type:"inkriver:article-link"');
    expect(document).toContain('window.parent.postMessage({type:"inkriver:article-image"');
    expect(document).toContain('window.parent.postMessage({type:"inkriver:article-swipe"');
    expect(document).toContain('message.type==="inkriver:article-image-focus"');
    expect(document).toContain('message.type==="inkriver:article-text-size"');
    expect(document).toContain("[16,18,22].includes(message.fontSize)");
    expect(document).toContain('style="--article-font-size:16px"');
    expect(document).toContain("font-size:var(--article-font-size)");
    expect(document).toContain('window.parent.postMessage({type:"inkriver:article-height"');
    expect(document).toContain('document.addEventListener("click"');
    expect(document).toContain('document.addEventListener("keydown"');
    expect(document).toContain('document.addEventListener("touchstart"');
    expect(document).toContain('document.addEventListener("touchmove"');
    expect(document.indexOf('closest("img[data-zoomable-image]")')).toBeLessThan(
      document.indexOf('closest("a[data-external-href],a[data-internal-fragment]")'),
    );
    expect(document).toContain("new ResizeObserver(reportArticleHeight)");
    expect(document).toContain("html,body{overflow:hidden}");
    expect(document).toContain("@media(max-width:720px){body{padding:8px 18px 48px}}");
  });
});
