import { describe, expect, it, vi } from "vitest";
import { InkRiverApp, canOpenOriginal, detectPlatform, errorMessage } from "./app";
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
};

function fakeApi(overrides: Partial<InkRiverApi> = {}): InkRiverApi {
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
    deleteFeed: vi.fn(async () => ({ feedId: feed.id, deletedArticles: 1 })),
    ...overrides,
  };
}

async function flush(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 0));
}

async function mounted(
  api = fakeApi(),
  opener = vi.fn(async () => undefined),
  confirmer = vi.fn(() => true),
) {
  document.body.innerHTML = '<div id="app"></div>';
  const root = document.querySelector<HTMLElement>("#app")!;
  const app = new InkRiverApp(root, api, opener, confirmer);
  const initialization = app.init();
  expect(root.querySelector('[data-testid="loading"]')).not.toBeNull();
  await initialization;
  return { root, api, opener, confirmer };
}

describe("InkRiverApp", () => {
  it("renders the cached timeline without refreshing on startup", async () => {
    const { root, api } = await mounted();
    expect(root.textContent).toContain("Observer Mars au crépuscule");
    expect(api.listArticles).toHaveBeenCalledOnce();
    expect(api.refreshFeeds).not.toHaveBeenCalled();
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
    expect(firstRow.querySelectorAll("button")).toHaveLength(3);
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
    expect(root.querySelector('[data-testid="read-state"]')?.textContent).toContain("Lu");
    expect(root.querySelector('[data-action="toggle-read"]')?.textContent).toContain(
      "Marquer comme non lu",
    );
    expect(root.querySelector("[data-article-row-id]")?.classList).toContain("read");
    expect(root.querySelector<HTMLIFrameElement>(".article-content")?.srcdoc).toContain(
      "Mars prend une teinte orangée",
    );
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
    expect(root.querySelector('[data-testid="read-state"]')?.textContent).toContain("Lu");
  });

  it("marks the selected article unread and read again in both panels", async () => {
    const { root, api } = await mounted();
    root.querySelector<HTMLElement>("[data-article-id]")!.click();
    await flush();

    root.querySelector<HTMLElement>('[data-action="toggle-read"]')!.click();
    await flush();
    expect(api.setArticleRead).toHaveBeenNthCalledWith(2, "space::mars", false);
    expect(root.querySelector('[data-testid="read-state"]')?.textContent).toContain("Non lu");
    expect(root.querySelector('[data-action="toggle-read"]')?.textContent).toContain(
      "Marquer comme lu",
    );
    expect(root.querySelector("[data-article-row-id]")?.classList).toContain("unread");

    root.querySelector<HTMLElement>('[data-action="toggle-read"]')!.click();
    await flush();
    expect(api.setArticleRead).toHaveBeenNthCalledWith(3, "space::mars", true);
    expect(root.querySelector('[data-testid="read-state"]')?.textContent).toContain("Lu");
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
    expect(root.querySelector('[data-testid="read-state"]')?.textContent).toContain("Non lu");

    root.querySelector<HTMLElement>('[data-article-id="space::mars"]')!.click();
    await flush();
    expect(getArticle).toHaveBeenCalledTimes(1);
    expect(api.setArticleRead).toHaveBeenCalledTimes(2);
    expect(root.querySelector('[data-testid="read-state"]')?.textContent).toContain("Non lu");

    root.querySelector<HTMLElement>('[data-article-id="space::venus"]')!.click();
    await flush();
    root.querySelector<HTMLElement>('[data-article-id="space::mars"]')!.click();
    await flush();

    expect(getArticle).toHaveBeenCalledTimes(3);
    expect(api.setArticleRead).toHaveBeenNthCalledWith(3, "space::mars", true);
    expect(root.querySelector('[data-testid="read-state"]')?.textContent).toContain("Lu");
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
    expect(pendingButton.textContent).toContain("Enregistrement");

    finishUpdate!();
    await flush();
    expect(root.querySelector('[data-testid="read-state"]')?.textContent).toContain("Non lu");
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
    expect(root.querySelector('[data-testid="read-state"]')?.textContent).toContain("Lu");
    expect(root.querySelector("[data-article-row-id]")?.classList).toContain("read");
  });

  it("toggles a favorite from the reading panel", async () => {
    const { root, api } = await mounted();
    root.querySelector<HTMLElement>("[data-article-id]")!.click();
    await flush();
    root.querySelector<HTMLElement>('[data-action="favorite"]')!.click();
    await flush();
    expect(api.setArticleFavorite).toHaveBeenCalledWith("space::mars", true);
    expect(root.textContent).toContain("Retirer des favoris");
    expect(root.querySelector('[data-action="timeline-favorite"]')?.getAttribute("aria-pressed")).toBe("true");

    root.querySelector<HTMLElement>('[data-action="timeline-favorite"]')!.click();
    await flush();
    expect(api.setArticleFavorite).toHaveBeenNthCalledWith(2, "space::mars", false);
    expect(root.querySelector('[data-action="favorite"]')?.textContent).toContain(
      "Ajouter aux favoris",
    );
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
    root.querySelector<HTMLElement>('[data-action="toggle-feed"]')!.click();
    await flush();
    expect(api.setFeedActive).toHaveBeenCalledWith("stable-feed-id", false);
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
    expect(root.textContent).toContain(summary.title);
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
    expect(detectPlatform("https://medium.com/feed/@inkriver")).toBe("medium");
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
