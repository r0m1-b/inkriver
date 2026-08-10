import type { ReaderApi } from "./api";
import type {
  ApiError,
  ArticleDetail,
  ArticleSummary,
  Feed,
  Platform,
  RefreshReport,
} from "./types";

type OpenOriginal = (url: string) => Promise<void>;

function escapeHtml(value: string): string {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#039;");
}

function displayDate(value: string | null): string {
  if (!value) return "Date inconnue";
  return new Intl.DateTimeFormat("fr-FR", { dateStyle: "medium" }).format(new Date(value));
}

function displayPlatform(platform: Platform): string {
  return platform === "other" ? "RSS" : platform[0]!.toUpperCase() + platform.slice(1);
}

export function detectPlatform(url: string): Platform {
  try {
    const host = new URL(url).hostname.toLowerCase();
    if (host === "medium.com" || host.endsWith(".medium.com")) return "medium";
    if (host === "substack.com" || host.endsWith(".substack.com")) return "substack";
  } catch {
    // Rust performs authoritative validation when the form is submitted.
  }
  return "other";
}

export function errorMessage(error: unknown): string {
  if (typeof error === "object" && error !== null && "message" in error) {
    return String((error as ApiError).message);
  }
  return String(error);
}

export function canOpenOriginal(article: ArticleDetail): boolean {
  return Boolean(article.url && article.contentKind !== "full");
}

export class ReaderApp {
  private articles: ArticleSummary[] = [];
  private feeds: Feed[] = [];
  private selected: ArticleDetail | null = null;
  private loading = true;
  private refreshing = false;
  private subscriptionsOpen = false;
  private error: string | null = null;
  private notice: string | null = null;

  constructor(
    private readonly root: HTMLElement,
    private readonly api: ReaderApi,
    private readonly openOriginal: OpenOriginal,
  ) {}

  async init(): Promise<void> {
    this.render();
    try {
      [this.articles, this.feeds] = await Promise.all([
        this.api.listArticles(),
        this.api.listFeeds(),
      ]);
    } catch (error) {
      this.error = errorMessage(error);
    } finally {
      this.loading = false;
      this.render();
    }
  }

  private async selectArticle(articleId: string): Promise<void> {
    this.error = null;
    try {
      this.selected = await this.api.getArticle(articleId);
      if (!this.selected.isRead) {
        await this.api.setArticleRead(articleId, true);
        this.selected.isRead = true;
        const summary = this.articles.find((article) => article.id === articleId);
        if (summary) summary.isRead = true;
      }
    } catch (error) {
      this.error = errorMessage(error);
    }
    this.render();
  }

  private async toggleFavorite(): Promise<void> {
    if (!this.selected) return;
    const nextValue = !this.selected.isFavorite;
    try {
      await this.api.setArticleFavorite(this.selected.id, nextValue);
      this.selected.isFavorite = nextValue;
      const summary = this.articles.find((article) => article.id === this.selected?.id);
      if (summary) summary.isFavorite = nextValue;
    } catch (error) {
      this.error = errorMessage(error);
    }
    this.render();
  }

  private async refresh(): Promise<void> {
    this.refreshing = true;
    this.error = null;
    this.notice = null;
    this.render();
    try {
      const report = await this.api.refreshFeeds();
      this.articles = await this.api.listArticles();
      this.notice = this.refreshNotice(report);
      if (this.selected) {
        this.selected = await this.api.getArticle(this.selected.id).catch(() => null);
      }
    } catch (error) {
      this.error = errorMessage(error);
    } finally {
      this.refreshing = false;
      this.render();
    }
  }

  private refreshNotice(report: RefreshReport): string {
    const result = `${report.insertedArticles} nouveau(x), ${report.updatedArticles} actualisé(s)`;
    return report.errors.length === 0
      ? `Actualisation terminée : ${result}.`
      : `Actualisation partielle : ${result}, ${report.errors.length} flux en erreur.`;
  }

  private async submitFeed(form: HTMLFormElement): Promise<void> {
    const formData = new FormData(form);
    const url = String(formData.get("url") ?? "");
    const platform = String(formData.get("platform") ?? "") as Platform;
    this.error = null;
    try {
      await this.api.addFeed(url, platform);
      this.feeds = await this.api.listFeeds();
      this.notice = "Abonnement ajouté. Utilisez Actualiser pour télécharger ses articles.";
      form.reset();
    } catch (error) {
      this.error = errorMessage(error);
    }
    this.render();
  }

  private async toggleFeed(feedId: string, isActive: boolean): Promise<void> {
    this.error = null;
    try {
      await this.api.setFeedActive(feedId, isActive);
      this.feeds = await this.api.listFeeds();
    } catch (error) {
      this.error = errorMessage(error);
    }
    this.render();
  }

  private async openSelectedOriginal(): Promise<void> {
    if (!this.selected?.url) return;
    try {
      const url = new URL(this.selected.url);
      if (!(["http:", "https:"] as string[]).includes(url.protocol)) {
        throw new Error("Seuls les liens HTTP(S) peuvent être ouverts.");
      }
      await this.openOriginal(url.toString());
    } catch (error) {
      this.error = errorMessage(error);
      this.render();
    }
  }

  private renderArticleList(): string {
    if (this.loading) return '<div class="state" data-testid="loading">Chargement du cache…</div>';
    if (this.articles.length === 0) {
      return '<div class="state" data-testid="empty">Aucun article enregistré.<button class="text-button" data-action="subscriptions">Ajouter un abonnement</button></div>';
    }
    return this.articles
      .map(
        (article) => `<button class="article-row ${article.isRead ? "read" : "unread"} ${this.selected?.id === article.id ? "selected" : ""}" data-article-id="${escapeHtml(article.id)}">
          <span class="row-top"><span class="source ${article.source}">${displayPlatform(article.source)}</span><time>${displayDate(article.publishedAt)}</time></span>
          <strong>${escapeHtml(article.title ?? "Sans titre")}</strong>
          <span class="byline">${escapeHtml(article.author ?? "Auteur inconnu")}</span>
          ${article.isFavorite ? '<span class="favorite-mark" aria-label="Favori">★</span>' : ""}
        </button>`,
      )
      .join("");
  }

  private renderReader(): string {
    if (!this.selected) {
      return '<div class="reader-placeholder"><span>R</span><p>Sélectionnez un article dans la chronologie.</p></div>';
    }
    const article = this.selected;
    const originalButton = canOpenOriginal(article)
      ? '<button class="primary" data-action="open-original">Lire l’original ↗</button>'
      : "";
    const content = article.content
      ? '<iframe class="article-content" title="Contenu de l’article" sandbox=""></iframe>'
      : '<p class="missing-content">Le flux ne fournit pas de contenu pour cet article.</p>';
    return `<article class="reader-article">
      <header><span class="source ${article.source}">${displayPlatform(article.source)}</span>
      <h1>${escapeHtml(article.title ?? "Sans titre")}</h1>
      <p>${escapeHtml(article.author ?? "Auteur inconnu")} · ${displayDate(article.publishedAt)}</p>
      <div class="reader-actions"><button data-action="favorite" aria-pressed="${article.isFavorite}">${article.isFavorite ? "★ Retirer des favoris" : "☆ Ajouter aux favoris"}</button>${originalButton}</div></header>
      ${content}
    </article>`;
  }

  private renderSubscriptions(): string {
    if (!this.subscriptionsOpen) return "";
    const feeds = this.feeds.length
      ? this.feeds
          .map(
            (feed) => `<li><div><strong>${displayPlatform(feed.platform)}</strong><span>${escapeHtml(feed.url)}</span></div><button data-feed-id="${escapeHtml(feed.id)}" data-next-active="${!feed.isActive}">${feed.isActive ? "Désactiver" : "Réactiver"}</button></li>`,
          )
          .join("")
      : '<li class="state">Aucun abonnement.</li>';
    return `<div class="modal-backdrop"><section class="subscriptions" role="dialog" aria-modal="true" aria-labelledby="subscriptions-title">
      <header><div><span class="eyebrow">Sources</span><h2 id="subscriptions-title">Abonnements</h2></div><button class="icon-button" data-action="close-subscriptions" aria-label="Fermer">×</button></header>
      <form id="feed-form"><label>URL du flux<input name="url" type="url" required placeholder="https://publication.substack.com/feed"></label><label>Plateforme<select name="platform"><option value="other">RSS / autre</option><option value="medium">Medium</option><option value="substack">Substack</option></select></label><button class="primary" type="submit">Ajouter</button></form>
      <ul class="feed-list">${feeds}</ul>
    </section></div>`;
  }

  render(): void {
    this.root.innerHTML = `<div class="shell">
      <header class="topbar"><div class="brand"><span>R</span><div><strong>Reader</strong><small>Medium + Substack</small></div></div><div class="top-actions"><button data-action="subscriptions">Abonnements</button><button class="primary" data-action="refresh" ${this.refreshing ? "disabled" : ""}>${this.refreshing ? "Actualisation…" : "Actualiser"}</button></div></header>
      ${this.error ? `<div class="banner error" role="alert">${escapeHtml(this.error)}</div>` : ""}
      ${this.notice ? `<div class="banner notice" role="status">${escapeHtml(this.notice)}</div>` : ""}
      <main><aside class="timeline" aria-label="Chronologie">${this.renderArticleList()}</aside><section class="reader">${this.renderReader()}</section></main>
      ${this.renderSubscriptions()}
    </div>`;

    const frame = this.root.querySelector<HTMLIFrameElement>(".article-content");
    if (frame && this.selected?.content) {
      frame.srcdoc = `<!doctype html><meta charset="utf-8"><meta name="color-scheme" content="light dark"><style>body{font:18px/1.75 Georgia,serif;max-width:720px;margin:0 auto;padding:8px 32px 64px;color:#292621;background:transparent}img{max-width:100%;height:auto}a{color:#a84d2f}pre{white-space:pre-wrap}@media(prefers-color-scheme:dark){body{color:#e8e1d8}}</style>${this.selected.content}`;
    }
    this.bindEvents();
  }

  private bindEvents(): void {
    this.root.querySelectorAll<HTMLElement>("[data-article-id]").forEach((element) => {
      element.addEventListener("click", () => void this.selectArticle(element.dataset.articleId!));
    });
    this.root.querySelectorAll<HTMLElement>('[data-action="subscriptions"]').forEach((element) => {
      element.addEventListener("click", () => {
        this.subscriptionsOpen = true;
        this.render();
      });
    });
    this.root.querySelector<HTMLElement>('[data-action="close-subscriptions"]')?.addEventListener("click", () => {
      this.subscriptionsOpen = false;
      this.render();
    });
    this.root.querySelector<HTMLElement>('[data-action="refresh"]')?.addEventListener("click", () => void this.refresh());
    this.root.querySelector<HTMLElement>('[data-action="favorite"]')?.addEventListener("click", () => void this.toggleFavorite());
    this.root.querySelector<HTMLElement>('[data-action="open-original"]')?.addEventListener("click", () => void this.openSelectedOriginal());
    this.root.querySelector<HTMLFormElement>("#feed-form")?.addEventListener("submit", (event) => {
      event.preventDefault();
      void this.submitFeed(event.currentTarget as HTMLFormElement);
    });
    const urlInput = this.root.querySelector<HTMLInputElement>('input[name="url"]');
    const platformSelect = this.root.querySelector<HTMLSelectElement>('select[name="platform"]');
    urlInput?.addEventListener("input", () => {
      if (platformSelect) platformSelect.value = detectPlatform(urlInput.value);
    });
    this.root.querySelectorAll<HTMLElement>("[data-feed-id]").forEach((element) => {
      element.addEventListener("click", () =>
        void this.toggleFeed(element.dataset.feedId!, element.dataset.nextActive === "true"),
      );
    });
  }
}
