import { describe, expect, it, vi } from "vitest";
import { ReaderApp, canOpenOriginal, detectPlatform, errorMessage } from "./app";
import type { ReaderApi } from "./api";
import type { ArticleDetail, ArticleSummary, Feed, RefreshReport } from "./types";

const summary: ArticleSummary = {
  id: "space::mars",
  feedId: "space",
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

const feed: Feed = {
  id: "stable-feed-id",
  platform: "substack",
  url: "https://space.substack.com/feed",
  isActive: true,
};

function fakeApi(overrides: Partial<ReaderApi> = {}): ReaderApi {
  return {
    listArticles: vi.fn(async () => [structuredClone(summary)]),
    getArticle: vi.fn(async () => structuredClone(detail)),
    refreshFeeds: vi.fn(async (): Promise<RefreshReport> => ({
      activeFeeds: 1,
      collectedArticles: 1,
      insertedArticles: 0,
      updatedArticles: 1,
      errors: [],
    })),
    setArticleRead: vi.fn(async () => undefined),
    setArticleFavorite: vi.fn(async () => undefined),
    listFeeds: vi.fn(async () => [structuredClone(feed)]),
    addFeed: vi.fn(async () => structuredClone(feed)),
    setFeedActive: vi.fn(async () => structuredClone(feed)),
    ...overrides,
  };
}

async function flush(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 0));
}

async function mounted(api = fakeApi(), opener = vi.fn(async () => undefined)) {
  document.body.innerHTML = '<div id="app"></div>';
  const root = document.querySelector<HTMLElement>("#app")!;
  const app = new ReaderApp(root, api, opener);
  const initialization = app.init();
  expect(root.querySelector('[data-testid="loading"]')).not.toBeNull();
  await initialization;
  return { root, api, opener };
}

describe("ReaderApp", () => {
  it("renders the cached timeline without refreshing on startup", async () => {
    const { root, api } = await mounted();
    expect(root.textContent).toContain("Observer Mars au crépuscule");
    expect(api.listArticles).toHaveBeenCalledOnce();
    expect(api.refreshFeeds).not.toHaveBeenCalled();
  });

  it("renders an empty state that opens subscription management", async () => {
    const { root } = await mounted(fakeApi({ listArticles: vi.fn(async () => []) }));
    expect(root.querySelector('[data-testid="empty"]')).not.toBeNull();
    root.querySelector<HTMLElement>('[data-action="subscriptions"]')!.click();
    expect(root.querySelector('[role="dialog"]')).not.toBeNull();
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
    expect(root.querySelector<HTMLIFrameElement>(".article-content")?.srcdoc).toContain(
      "Mars prend une teinte orangée",
    );
  });

  it("toggles a favorite from the reading panel", async () => {
    const { root, api } = await mounted();
    root.querySelector<HTMLElement>("[data-article-id]")!.click();
    await flush();
    root.querySelector<HTMLElement>('[data-action="favorite"]')!.click();
    await flush();
    expect(api.setArticleFavorite).toHaveBeenCalledWith("space::mars", true);
    expect(root.textContent).toContain("Retirer des favoris");
  });

  it("reports a partial refresh while retaining cached articles", async () => {
    const api = fakeApi({
      refreshFeeds: vi.fn(async () => ({
        activeFeeds: 2,
        collectedArticles: 1,
        insertedArticles: 1,
        updatedArticles: 0,
        errors: [{ feedId: "bread", feedUrl: "https://bread.example", stage: "HTTP request", message: "offline" }],
      })),
    });
    const { root } = await mounted(api);
    root.querySelector<HTMLElement>('[data-action="refresh"]')!.click();
    await flush();
    expect(root.textContent).toContain("Actualisation partielle");
    expect(root.textContent).toContain("Observer Mars au crépuscule");
  });

  it("adds and deactivates subscriptions without triggering a refresh", async () => {
    const { root, api } = await mounted();
    root.querySelector<HTMLElement>('[data-action="subscriptions"]')!.click();
    const input = root.querySelector<HTMLInputElement>('input[name="url"]')!;
    input.value = "https://notes.medium.com/feed";
    input.dispatchEvent(new Event("input"));
    expect(root.querySelector<HTMLSelectElement>('select[name="platform"]')!.value).toBe("medium");
    root.querySelector<HTMLFormElement>("#feed-form")!.dispatchEvent(new Event("submit", { cancelable: true }));
    await flush();
    expect(api.addFeed).toHaveBeenCalledWith("https://notes.medium.com/feed", "medium");
    expect(api.refreshFeeds).not.toHaveBeenCalled();
    root.querySelector<HTMLElement>("[data-feed-id]")!.click();
    await flush();
    expect(api.setFeedActive).toHaveBeenCalledWith("stable-feed-id", false);
  });

  it("opens an excerpt original through the injected opener", async () => {
    const { root, opener } = await mounted();
    root.querySelector<HTMLElement>("[data-article-id]")!.click();
    await flush();
    root.querySelector<HTMLElement>('[data-action="open-original"]')!.click();
    await flush();
    expect(opener).toHaveBeenCalledWith("https://space.example/mars");
  });
});

describe("view helpers", () => {
  it("detects known platforms without accepting lookalike domains", () => {
    expect(detectPlatform("https://medium.com/feed/@reader")).toBe("medium");
    expect(detectPlatform("https://letters.substack.com/feed")).toBe("substack");
    expect(detectPlatform("https://substack.com.example/feed")).toBe("other");
  });

  it("shows original links for excerpts, missing and legacy unknown content", () => {
    expect(canOpenOriginal(detail)).toBe(true);
    expect(canOpenOriginal({ ...detail, contentKind: "missing" })).toBe(true);
    expect(canOpenOriginal({ ...detail, contentKind: "unknown" })).toBe(true);
    expect(canOpenOriginal({ ...detail, contentKind: "full" })).toBe(false);
  });

  it("extracts structured errors and falls back to strings", () => {
    expect(errorMessage({ code: "storage", message: "Base indisponible" })).toBe("Base indisponible");
    expect(errorMessage("offline")).toBe("offline");
  });
});
