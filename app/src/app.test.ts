import { describe, expect, it, vi } from "vitest";
import {
  InkRiverApp,
  articleSourceHost,
  buildArticleDocument,
  canOpenOriginal,
  detectPlatform,
  errorMessage,
  prepareArticleContent,
  resolveExternalArticleUrl,
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
      autoArchivedArticles: 0,
      extractedArticles: 0,
      extractionFailedArticles: 0,
      extractionSkippedArticles: 0,
      errors: [],
    })),
    setArticleRead: vi.fn(async () => undefined),
    setArticleFavorite: vi.fn(async () => undefined),
    archiveArticle: vi.fn(async () => undefined),
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
  it("renders the InkRiver logo in the application header", async () => {
    const { root } = await mounted();
    const logo = root.querySelector<HTMLImageElement>(".brand-logo");

    expect(logo?.getAttribute("src")).toBe("/inkriver-logo.png");
    expect(logo?.getAttribute("alt")).toBe("");
    expect(root.querySelector(".brand small")?.textContent).toBe("All your feeds. One flow.");
  });

  it("renders the cached timeline without refreshing on startup", async () => {
    const { root, api } = await mounted();
    expect(root.textContent).toContain("Observer Mars au crépuscule");
    expect(api.listArticles).toHaveBeenCalledOnce();
    expect(api.refreshFeeds).not.toHaveBeenCalled();
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
    expect(firstRow.querySelectorAll("button")).toHaveLength(3);
  });

  it("renders a platform icon while retaining the source label", async () => {
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
    expect(root.querySelector('[data-article-row-id="source::medium"] .source')?.textContent).toBe(
      "Medium",
    );
    expect(root.querySelector('[data-article-row-id="source::rss"] .source')?.textContent).toBe(
      "RSS",
    );
    expect(root.querySelector('[data-source-icon="substack"]')?.getAttribute("aria-hidden")).toBe(
      "true",
    );
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
    expect(root.querySelector('[data-testid="read-state"]')?.textContent).toContain("Lu");
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
    expect(root.querySelector('[data-action="toggle-read"]')?.getAttribute("title")).toBe(
      "Marquer comme lu",
    );
    expect(root.querySelector('[data-action="toggle-read"]')?.getAttribute("aria-pressed")).toBe("false");
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
    expect(pendingButton.textContent?.trim()).toBe("");
    expect(pendingButton.getAttribute("aria-busy")).toBe("true");
    expect(pendingButton.getAttribute("title")).toBe("Marquer comme non lu");

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
      "1 ancien(s) supprimé(s) automatiquement",
    );
    expect(root.querySelector('[role="status"]')?.textContent).toContain("2 extrait(s)");
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
    expect(root.textContent).toContain("Actualisation partielle");
    expect(root.textContent).toContain("2 ancien(s) supprimé(s) automatiquement");
    expect(root.textContent).toContain("Observer Mars au crépuscule");
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

  it("adds and deactivates subscriptions without triggering a refresh", async () => {
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
    expect(missing.root.querySelector('[data-action="open-source"]')).toBeNull();
    expect(missing.root.querySelector('[data-action="open-original"]')).toBeNull();
    expect(missing.opener).not.toHaveBeenCalled();

    const invalidApi = fakeApi({
      getArticle: vi.fn(async () => ({ ...structuredClone(detail), url: "mailto:author@example.com" })),
    });
    const invalid = await mounted(invalidApi);
    invalid.root.querySelector<HTMLElement>("[data-article-id]")!.click();
    await flush();
    expect(invalid.root.querySelector(".article-source")?.textContent).toContain("Source non prise en charge");
    expect(invalid.root.querySelector('[data-action="open-source"]')).toBeNull();
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

  it("builds a nonce-protected iframe bridge for links", () => {
    const document = buildArticleDocument(
      '<a href="https://example.com/read">Read</a>',
      "test-nonce",
    );
    expect(document).toContain("script-src 'nonce-test-nonce'");
    expect(document).toContain('<script nonce="test-nonce">');
    expect(document).toContain('window.parent.postMessage({type:"inkriver:article-link"');
    expect(document).toContain('document.addEventListener("click"');
  });
});
