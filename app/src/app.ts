import type { InkRiverApi } from "./api";
import type {
  ApiError,
  ArticleDetail,
  ArticleSummary,
  Feed,
  Platform,
  RefreshReport,
} from "./types";

type OpenOriginal = (url: string) => Promise<void>;
type ConfirmDeletion = (message: string) => boolean;

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

function favoriteIcon(isFavorite: boolean): string {
  return `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="m12 2.75 2.86 5.8 6.4.93-4.63 4.51 1.09 6.37L12 17.35l-5.72 3.01 1.09-6.37-4.63-4.51 6.4-.93L12 2.75Z" ${isFavorite ? 'fill="currentColor"' : 'fill="none" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round"'}/></svg>`;
}

function readIcon(isRead: boolean): string {
  return isRead
    ? '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M3 10 12 3l9 7v10H3V10Z" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round"/><path d="m3 10 9 7 9-7" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round"/></svg>'
    : '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M3 6h18v13H3V6Z" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round"/><path d="m3 7 9 7 9-7" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round"/></svg>';
}

export class InkRiverApp {
  private articles: ArticleSummary[] = [];
  private feeds: Feed[] = [];
  private selected: ArticleDetail | null = null;
  private loading = true;
  private refreshing = false;
  private readonly updatingReadArticleIds = new Set<string>();
  private readonly updatingFavoriteArticleIds = new Set<string>();
  private subscriptionsOpen = false;
  private deletingFeedId: string | null = null;
  private error: string | null = null;
  private notice: string | null = null;

  constructor(
    private readonly root: HTMLElement,
    private readonly api: InkRiverApi,
    private readonly openOriginal: OpenOriginal,
    private readonly confirmDeletion: ConfirmDeletion,
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
    if (this.selected?.id === articleId) return;
    this.error = null;
    try {
      this.selected = await this.api.getArticle(articleId);
      if (!this.selected.isRead) {
        await this.setArticleReadState(articleId, true);
      }
    } catch (error) {
      this.error = errorMessage(error);
    }
    this.render();
  }

  private async toggleFavorite(): Promise<void> {
    if (!this.selected) return;
    await this.setArticleFavoriteState(this.selected.id, !this.selected.isFavorite);
  }

  private async setArticleFavoriteState(articleId: string, isFavorite: boolean): Promise<void> {
    if (this.updatingFavoriteArticleIds.has(articleId)) return;
    this.updatingFavoriteArticleIds.add(articleId);
    this.error = null;
    this.render();
    try {
      await this.api.setArticleFavorite(articleId, isFavorite);
      if (this.selected?.id === articleId) this.selected.isFavorite = isFavorite;
      const summary = this.articles.find((article) => article.id === articleId);
      if (summary) summary.isFavorite = isFavorite;
    } catch (error) {
      this.error = errorMessage(error);
    } finally {
      this.updatingFavoriteArticleIds.delete(articleId);
      this.render();
    }
  }

  private async toggleReadState(): Promise<void> {
    if (!this.selected) return;
    await this.setArticleReadState(this.selected.id, !this.selected.isRead);
  }

  private async setArticleReadState(articleId: string, isRead: boolean): Promise<void> {
    if (this.updatingReadArticleIds.has(articleId)) return;
    this.updatingReadArticleIds.add(articleId);
    this.error = null;
    this.render();
    try {
      await this.api.setArticleRead(articleId, isRead);
      if (this.selected?.id === articleId) this.selected.isRead = isRead;
      const summary = this.articles.find((article) => article.id === articleId);
      if (summary) summary.isRead = isRead;
    } catch (error) {
      this.error = errorMessage(error);
    } finally {
      this.updatingReadArticleIds.delete(articleId);
      this.render();
    }
  }

  private async toggleTimelineFavorite(articleId: string): Promise<void> {
    const article = this.articles.find((candidate) => candidate.id === articleId);
    if (article) await this.setArticleFavoriteState(articleId, !article.isFavorite);
  }

  private async toggleTimelineReadState(articleId: string): Promise<void> {
    const article = this.articles.find((candidate) => candidate.id === articleId);
    if (article) await this.setArticleReadState(articleId, !article.isRead);
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

  private async deleteFeed(feedId: string): Promise<void> {
    const feed = this.feeds.find((candidate) => candidate.id === feedId);
    if (!feed) return;
    const confirmed = this.confirmDeletion(
      `Supprimer l’abonnement « ${feed.url} » ? Ses articles, favoris et états de lecture seront définitivement supprimés.`,
    );
    if (!confirmed) return;

    this.deletingFeedId = feedId;
    this.error = null;
    this.notice = null;
    this.render();
    try {
      const result = await this.api.deleteFeed(feedId);
      this.feeds = this.feeds.filter((candidate) => candidate.id !== feedId);
      this.articles = this.articles.filter((article) => article.feedId !== feedId);
      if (this.selected?.feedId === feedId) this.selected = null;
      [this.feeds, this.articles] = await Promise.all([
        this.api.listFeeds(),
        this.api.listArticles(),
      ]);
      const label = result.deletedArticles > 1 ? "articles supprimés" : "article supprimé";
      this.notice = `Abonnement supprimé avec ${result.deletedArticles} ${label}.`;
    } catch (error) {
      this.error = errorMessage(error);
    } finally {
      this.deletingFeedId = null;
      this.render();
    }
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
      .map((article) => {
        const title = article.title ?? "Sans titre";
        const favoriteAction = article.isFavorite ? "Retirer des favoris" : "Ajouter aux favoris";
        const readAction = article.isRead ? "Marquer comme non lu" : "Marquer comme lu";
        const favoritePending = this.updatingFavoriteArticleIds.has(article.id);
        const readPending = this.updatingReadArticleIds.has(article.id);
        return `<article class="article-row ${article.isRead ? "read" : "unread"} ${this.selected?.id === article.id ? "selected" : ""}" data-article-row-id="${escapeHtml(article.id)}">
          <button type="button" class="article-select" data-action="select-article" data-article-id="${escapeHtml(article.id)}">
          <span class="row-top"><span class="source ${article.source}">${displayPlatform(article.source)}</span><time>${displayDate(article.publishedAt)}</time></span>
          <strong>${escapeHtml(title)}</strong>
          <span class="byline">${escapeHtml(article.author ?? "Auteur inconnu")}</span>
          </button>
          <span class="article-row-actions">
            <button type="button" class="article-icon-button favorite ${article.isFavorite ? "active" : ""}" data-action="timeline-favorite" data-state-article-id="${escapeHtml(article.id)}" aria-label="${escapeHtml(`${favoriteAction} : ${title}`)}" title="${favoriteAction}" aria-pressed="${article.isFavorite}" aria-busy="${favoritePending}" ${favoritePending ? "disabled" : ""}>${favoriteIcon(article.isFavorite)}</button>
            <button type="button" class="article-icon-button read-state-icon ${article.isRead ? "active" : ""}" data-action="timeline-read" data-state-article-id="${escapeHtml(article.id)}" aria-label="${escapeHtml(`${readAction} : ${title}`)}" title="${readAction}" aria-pressed="${article.isRead}" aria-busy="${readPending}" ${readPending ? "disabled" : ""}>${readIcon(article.isRead)}</button>
          </span>
        </article>`;
      })
      .join("");
  }

  private renderReader(): string {
    if (!this.selected) {
      return '<div class="reader-placeholder"><span>IR</span><p>Sélectionnez un article dans la chronologie.</p></div>';
    }
    const article = this.selected;
    const readPending = this.updatingReadArticleIds.has(article.id);
    const favoritePending = this.updatingFavoriteArticleIds.has(article.id);
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
      <div class="read-state" data-testid="read-state">État : <strong>${article.isRead ? "Lu" : "Non lu"}</strong></div>
      <div class="reader-actions"><button data-action="toggle-read" aria-busy="${readPending}" ${readPending ? "disabled" : ""}>${readPending ? "Enregistrement…" : article.isRead ? "Marquer comme non lu" : "Marquer comme lu"}</button><button data-action="favorite" aria-pressed="${article.isFavorite}" aria-busy="${favoritePending}" ${favoritePending ? "disabled" : ""}>${favoritePending ? "Enregistrement…" : article.isFavorite ? "★ Retirer des favoris" : "☆ Ajouter aux favoris"}</button>${originalButton}</div></header>
      ${content}
    </article>`;
  }

  private renderSubscriptions(): string {
    if (!this.subscriptionsOpen) return "";
    const feeds = this.feeds.length
      ? this.feeds
          .map(
            (feed) => `<li><div class="feed-details"><strong>${displayPlatform(feed.platform)}</strong><span>${escapeHtml(feed.url)}</span></div><div class="feed-actions"><button data-action="toggle-feed" data-feed-id="${escapeHtml(feed.id)}" data-next-active="${!feed.isActive}" ${this.refreshing || this.deletingFeedId !== null ? "disabled" : ""}>${feed.isActive ? "Désactiver" : "Réactiver"}</button><button class="danger" data-action="delete-feed" data-feed-id="${escapeHtml(feed.id)}" ${this.refreshing || this.deletingFeedId !== null ? "disabled" : ""}>${this.deletingFeedId === feed.id ? "Suppression…" : "Supprimer"}</button></div></li>`,
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
      <header class="topbar"><div class="brand"><span>IR</span><div><strong>InkRiver</strong><small>Medium + Substack</small></div></div><div class="top-actions"><button data-action="subscriptions">Abonnements</button><button class="primary" data-action="refresh" ${this.refreshing ? "disabled" : ""}>${this.refreshing ? "Actualisation…" : "Actualiser"}</button></div></header>
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
    this.root.querySelectorAll<HTMLElement>('[data-action="select-article"]').forEach((element) => {
      element.addEventListener("click", () => void this.selectArticle(element.dataset.articleId!));
    });
    this.root.querySelectorAll<HTMLElement>('[data-action="timeline-favorite"]').forEach((element) => {
      element.addEventListener("click", () =>
        void this.toggleTimelineFavorite(element.dataset.stateArticleId!),
      );
    });
    this.root.querySelectorAll<HTMLElement>('[data-action="timeline-read"]').forEach((element) => {
      element.addEventListener("click", () =>
        void this.toggleTimelineReadState(element.dataset.stateArticleId!),
      );
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
    this.root.querySelector<HTMLElement>('[data-action="toggle-read"]')?.addEventListener("click", () => void this.toggleReadState());
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
    this.root.querySelectorAll<HTMLElement>('[data-action="toggle-feed"]').forEach((element) => {
      element.addEventListener("click", () =>
        void this.toggleFeed(element.dataset.feedId!, element.dataset.nextActive === "true"),
      );
    });
    this.root.querySelectorAll<HTMLElement>('[data-action="delete-feed"]').forEach((element) => {
      element.addEventListener("click", () => void this.deleteFeed(element.dataset.feedId!));
    });
  }
}
