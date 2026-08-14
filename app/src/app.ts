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
type ArticleView = "all" | "favorites";
type MainView = "articles" | "feeds";
const ARTICLE_LINK_MESSAGE = "inkriver:article-link";

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

function displayDateTime(value: string | null): string {
  if (!value) return "Jamais";
  return new Intl.DateTimeFormat("fr-FR", {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(new Date(value));
}

function displayPlatform(platform: Platform): string {
  return platform === "other" ? "RSS" : platform[0]!.toUpperCase() + platform.slice(1);
}

function sourceIcon(platform: Platform): string {
  if (platform === "medium") {
    return '<svg class="source-icon" data-source-icon="medium" viewBox="0 0 24 24" aria-hidden="true"><path d="M4.21 0A4.201 4.201 0 0 0 0 4.21v15.58A4.201 4.201 0 0 0 4.21 24h15.58A4.201 4.201 0 0 0 24 19.79v-1.093c-.137.013-.278.02-.422.02-2.577 0-4.027-2.146-4.09-4.832a7.592 7.592 0 0 1 .022-.708c.093-1.186.475-2.241 1.105-3.022a3.885 3.885 0 0 1 1.395-1.1c.468-.237 1.127-.367 1.664-.367h.023c.101 0 .202.004.303.01V4.211A4.201 4.201 0 0 0 19.79 0Zm.198 5.583h4.165l3.588 8.435 3.59-8.435h3.864v.146l-.019.004c-.705.16-1.063.397-1.063 1.254h-.003l.003 10.274c.06.676.424.885 1.063 1.03l.02.004v.145h-4.923v-.145l.019-.005c.639-.144.994-.353 1.054-1.03V7.267l-4.745 11.15h-.261L6.15 7.569v9.445c0 .857.358 1.094 1.063 1.253l.02.004v.147H4.405v-.147l.019-.004c.705-.16 1.065-.397 1.065-1.253V6.987c0-.857-.358-1.094-1.064-1.254l-.018-.004zm19.25 3.668c-1.086.023-1.733 1.323-1.813 3.124H24V9.298a1.378 1.378 0 0 0-.342-.047Zm-1.862 3.632c-.1 1.756.86 3.239 2.204 3.634v-3.634z" fill="currentColor"/></svg>';
  }
  if (platform === "substack") {
    return '<svg class="source-icon" data-source-icon="substack" viewBox="0 0 24 24" aria-hidden="true"><path d="M22.539 8.242H1.46V5.406h21.08v2.836zM1.46 10.812V24L12 18.11 22.54 24V10.812H1.46zM22.54 0H1.46v2.836h21.08V0z" fill="currentColor"/></svg>';
  }
  return '<svg class="source-icon" data-source-icon="rss" viewBox="0 0 24 24" aria-hidden="true"><circle cx="6" cy="18" r="2" fill="currentColor"/><path d="M4 11a9 9 0 0 1 9 9M4 5a15 15 0 0 1 15 15" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round"/></svg>';
}

function renderSourceBadge(platform: Platform): string {
  return `<span class="source-identity"><span class="source-logo ${platform}">${sourceIcon(platform)}</span><span class="source ${platform}">${displayPlatform(platform)}</span></span>`;
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

export function articleSourceHost(rawUrl: string | null): string | null {
  if (!rawUrl) return null;
  try {
    const url = new URL(rawUrl);
    return url.protocol === "http:" || url.protocol === "https:" ? url.host : null;
  } catch {
    return null;
  }
}

export function canOpenOriginal(article: ArticleDetail): boolean {
  return Boolean(articleSourceHost(article.url) && article.contentKind !== "full");
}

export function resolveExternalArticleUrl(rawUrl: string, articleUrl?: string | null): string {
  let url: URL;
  try {
    url = articleUrl ? new URL(rawUrl, articleUrl) : new URL(rawUrl);
  } catch {
    throw new Error(`Lien invalide : ${rawUrl}`);
  }
  if (url.protocol !== "http:" && url.protocol !== "https:") {
    throw new Error("Seuls les liens HTTP(S) peuvent être ouverts.");
  }
  return url.toString();
}

export function prepareArticleContent(content: string): string {
  const document = new DOMParser().parseFromString(content, "text/html");
  document.querySelectorAll<HTMLAnchorElement>("a[href]").forEach((link) => {
    const href = link.getAttribute("href");
    link.removeAttribute("target");
    if (!href) return;
    if (href.startsWith("#")) {
      link.dataset.internalFragment = href.slice(1);
      link.setAttribute("href", "about:srcdoc#");
    } else {
      link.dataset.externalHref = href;
      link.setAttribute("href", "about:srcdoc#");
    }
  });
  return document.body.innerHTML;
}

export function buildArticleDocument(content: string, nonce: string): string {
  const preparedContent = prepareArticleContent(content);
  const bridgeScript = `document.addEventListener("click",function(event){var target=event.target;var link=target&&target.closest?target.closest("a[data-external-href],a[data-internal-fragment]"):null;if(!link)return;event.preventDefault();var href=link.getAttribute("data-external-href");if(href){window.parent.postMessage({type:"${ARTICLE_LINK_MESSAGE}",href:href},"*");return;}var fragment=link.getAttribute("data-internal-fragment");if(!fragment)return;var destination=document.getElementById(fragment)||document.getElementsByName(fragment)[0];if(destination)destination.scrollIntoView({block:"start",inline:"nearest"});},true);`;
  return `<!doctype html><html><head><meta charset="utf-8"><meta name="color-scheme" content="light dark"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src https: data:; style-src 'unsafe-inline'; script-src 'nonce-${nonce}'; base-uri 'none'; form-action 'none'"><style>body{font:18px/1.75 Georgia,serif;max-width:720px;margin:0 auto;padding:8px 32px 64px;color:#292621;background:transparent}img{max-width:100%;height:auto}a{color:#a84d2f}pre{white-space:pre-wrap}@media(prefers-color-scheme:dark){body{color:#e8e1d8}}</style><script nonce="${nonce}">${bridgeScript}</script></head><body>${preparedContent}</body></html>`;
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
  private articleView: ArticleView = "all";
  private mainView: MainView = "articles";
  private loading = true;
  private refreshing = false;
  private readonly updatingReadArticleIds = new Set<string>();
  private readonly updatingFavoriteArticleIds = new Set<string>();
  private addSubscriptionOpen = false;
  private deletingFeedId: string | null = null;
  private error: string | null = null;
  private notice: string | null = null;

  constructor(
    private readonly root: HTMLElement,
    private readonly api: InkRiverApi,
    private readonly openOriginal: OpenOriginal,
    private readonly confirmDeletion: ConfirmDeletion,
  ) {
    this.root.ownerDocument.defaultView?.addEventListener("message", (event) => {
      const frame = this.root.querySelector<HTMLIFrameElement>(".article-content");
      if (!frame || event.source !== frame.contentWindow) return;
      const message = event.data as { type?: unknown; href?: unknown } | null;
      if (
        !message ||
        message.type !== ARTICLE_LINK_MESSAGE ||
        typeof message.href !== "string"
      ) {
        return;
      }
      void this.openExternalUrl(message.href, this.selected?.url);
    });
  }

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
    if (this.selected?.id === articleId) this.scrollSelectedArticleIntoView();
  }

  private scrollSelectedArticleIntoView(): void {
    if (!this.selected) return;
    const selectedRow = Array.from(
      this.root.querySelectorAll<HTMLElement>("[data-article-row-id]"),
    ).find((row) => row.dataset.articleRowId === this.selected?.id);
    selectedRow?.scrollIntoView?.({ block: "nearest", inline: "nearest" });
  }

  private async toggleFavorite(): Promise<void> {
    if (!this.selected) return;
    await this.setArticleFavoriteState(this.selected.id, !this.selected.isFavorite);
  }

  private showArticleView(view: ArticleView): void {
    if (this.articleView === view) return;
    this.articleView = view;
    this.render();
    const timeline = this.root.querySelector<HTMLElement>(".timeline");
    if (timeline) timeline.scrollTop = 0;
    this.scrollSelectedArticleIntoView();
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
      [this.articles, this.feeds] = await Promise.all([
        this.api.listArticles(),
        this.api.listFeeds(),
      ]);
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
      : `Actualisation partielle : ${result}, ${report.errors.length} flux en erreur. Consultez la page Abonnements.`;
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
      this.addSubscriptionOpen = false;
      this.mainView = "feeds";
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

  private async openExternalUrl(rawUrl: string, articleUrl?: string | null): Promise<void> {
    try {
      await this.openOriginal(resolveExternalArticleUrl(rawUrl, articleUrl));
    } catch (error) {
      this.error = errorMessage(error);
      this.render();
    }
  }

  private async openSelectedOriginal(): Promise<void> {
    if (this.selected?.url) await this.openExternalUrl(this.selected.url);
  }

  private renderArticleList(): string {
    if (this.loading) return '<div class="state" data-testid="loading">Chargement du cache…</div>';
    if (this.articles.length === 0) {
      return '<div class="state" data-testid="empty">Aucun article enregistré.<button class="text-button" data-action="add-subscription">Ajouter un abonnement</button></div>';
    }
    const visibleArticles = this.articleView === "favorites"
      ? this.articles.filter((article) => article.isFavorite)
      : this.articles;
    if (visibleArticles.length === 0) {
      return '<div class="state" data-testid="favorites-empty"><strong>Aucun article favori.</strong><span>Utilisez l’étoile pour retrouver un article ici.</span></div>';
    }
    return visibleArticles
      .map((article) => {
        const title = article.title ?? "Sans titre";
        const favoriteAction = article.isFavorite ? "Retirer des favoris" : "Ajouter aux favoris";
        const readAction = article.isRead ? "Marquer comme non lu" : "Marquer comme lu";
        const favoritePending = this.updatingFavoriteArticleIds.has(article.id);
        const readPending = this.updatingReadArticleIds.has(article.id);
        return `<article class="article-row ${article.isRead ? "read" : "unread"} ${this.selected?.id === article.id ? "selected" : ""}" data-article-row-id="${escapeHtml(article.id)}">
          <button type="button" class="article-select" data-action="select-article" data-article-id="${escapeHtml(article.id)}">
          <span class="row-top">${renderSourceBadge(article.source)}<time>${displayDate(article.publishedAt)}</time></span>
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

  private renderArticleViews(): string {
    const favoriteCount = this.articles.filter((article) => article.isFavorite).length;
    return `<nav class="timeline-tabs" aria-label="Vues des articles" role="tablist">
      <button type="button" class="timeline-tab ${this.articleView === "all" ? "active" : ""}" data-action="article-view" data-article-view="all" role="tab" aria-selected="${this.articleView === "all"}">Tous</button>
      <button type="button" class="timeline-tab ${this.articleView === "favorites" ? "active" : ""}" data-action="article-view" data-article-view="favorites" role="tab" aria-selected="${this.articleView === "favorites"}">Favoris <span class="tab-count">${favoriteCount}</span></button>
    </nav>`;
  }

  private renderReader(): string {
    if (!this.selected) {
      return '<div class="reader-placeholder"><span>IR</span><p>Sélectionnez un article dans la chronologie.</p></div>';
    }
    const article = this.selected;
    const readPending = this.updatingReadArticleIds.has(article.id);
    const favoritePending = this.updatingFavoriteArticleIds.has(article.id);
    const sourceHost = articleSourceHost(article.url);
    const sourceLink = sourceHost && article.url
      ? `<div class="article-source">Source : <button type="button" class="article-source-link" data-action="open-source" title="${escapeHtml(article.url)}" aria-label="${escapeHtml(`Ouvrir la source ${sourceHost} dans le navigateur`)}">${escapeHtml(sourceHost)} ↗</button></div>`
      : `<div class="article-source">${article.url ? "Source non prise en charge" : "Source indisponible"}</div>`;
    const originalButton = canOpenOriginal(article)
      ? '<button class="primary" data-action="open-original">Lire l’original ↗</button>'
      : "";
    const content = article.content
      ? '<iframe class="article-content" title="Contenu de l’article" sandbox="allow-scripts"></iframe>'
      : '<p class="missing-content">Le flux ne fournit pas de contenu pour cet article.</p>';
    return `<article class="reader-article">
      <header>${renderSourceBadge(article.source)}
      <h1>${escapeHtml(article.title ?? "Sans titre")}</h1>
      <p>${escapeHtml(article.author ?? "Auteur inconnu")} · ${displayDate(article.publishedAt)}</p>
      ${sourceLink}
      <div class="read-state" data-testid="read-state">État : <strong>${article.isRead ? "Lu" : "Non lu"}</strong></div>
      <div class="reader-actions"><button data-action="toggle-read" aria-busy="${readPending}" ${readPending ? "disabled" : ""}>${readPending ? "Enregistrement…" : article.isRead ? "Marquer comme non lu" : "Marquer comme lu"}</button><button data-action="favorite" aria-pressed="${article.isFavorite}" aria-busy="${favoritePending}" ${favoritePending ? "disabled" : ""}>${favoritePending ? "Enregistrement…" : article.isFavorite ? "★ Retirer des favoris" : "☆ Ajouter aux favoris"}</button>${originalButton}</div></header>
      ${content}
    </article>`;
  }

  private renderFeedManagement(): string {
    const feeds = this.feeds.length
      ? this.feeds
          .map(
            (feed) => `<article class="feed-card ${feed.isActive ? "active" : "inactive"}" data-feed-card-id="${escapeHtml(feed.id)}">
              <header><div>${renderSourceBadge(feed.platform)}<span class="feed-status">${feed.isActive ? "Actif" : "Inactif"}</span></div><h2>${escapeHtml(feed.title ?? "Flux non actualisé")}</h2></header>
              <dl>
                <div><dt>URL du flux</dt><dd>${escapeHtml(feed.url)}</dd></div>
                <div><dt>Auteur</dt><dd>${escapeHtml(feed.author ?? "Inconnu")}</dd></div>
                <div class="feed-description"><dt>Description</dt><dd>${escapeHtml(feed.description ?? "Aucune description disponible.")}</dd></div>
                <div><dt>Dernière publication</dt><dd>${displayDateTime(feed.lastPublishedAt)}</dd></div>
                <div><dt>Dernière actualisation réussie</dt><dd>${displayDateTime(feed.lastSuccessAt)}</dd></div>
              </dl>
              ${feed.lastError ? `<section class="feed-error" aria-label="Dernière erreur"><strong>Dernière erreur · ${escapeHtml(feed.lastError.stage)}</strong><time>${displayDateTime(feed.lastError.occurredAt)}</time><p>${escapeHtml(feed.lastError.message)}</p></section>` : ""}
              <footer class="feed-actions"><button data-action="toggle-feed" data-feed-id="${escapeHtml(feed.id)}" data-next-active="${!feed.isActive}" ${this.refreshing || this.deletingFeedId !== null ? "disabled" : ""}>${feed.isActive ? "Désactiver" : "Réactiver"}</button><button class="danger" data-action="delete-feed" data-feed-id="${escapeHtml(feed.id)}" ${this.refreshing || this.deletingFeedId !== null ? "disabled" : ""}>${this.deletingFeedId === feed.id ? "Suppression…" : "Supprimer"}</button></footer>
            </article>`,
          )
          .join("")
      : '<div class="state" data-testid="feeds-empty">Aucun abonnement.<button class="text-button" data-action="add-subscription">Ajouter un abonnement</button></div>';
    return `<section class="feed-management" data-testid="feed-management"><header><div><span class="eyebrow">Sources</span><h1>Gestion des abonnements</h1><p>Consultez l’état des flux et leur dernier rafraîchissement.</p></div><button class="primary" data-action="add-subscription">Ajouter un abonnement</button></header><div class="feed-grid">${feeds}</div></section>`;
  }

  private renderAddSubscription(): string {
    if (!this.addSubscriptionOpen) return "";
    return `<div class="modal-backdrop"><section class="subscriptions add-subscription" role="dialog" aria-modal="true" aria-labelledby="subscriptions-title">
      <header><div><span class="eyebrow">Nouvelle source</span><h2 id="subscriptions-title">Ajouter un abonnement</h2></div><button class="icon-button" data-action="close-add-subscription" aria-label="Fermer">×</button></header>
      <form id="feed-form"><label>URL du flux<input name="url" type="url" required placeholder="https://publication.substack.com/feed"></label><label>Plateforme<select name="platform"><option value="other">RSS / autre</option><option value="medium">Medium</option><option value="substack">Substack</option></select></label><button class="primary" type="submit">Ajouter</button></form>
    </section></div>`;
  }

  render(): void {
    const timelineScrollTop =
      this.root.querySelector<HTMLElement>(".timeline")?.scrollTop;
    this.root.innerHTML = `<div class="shell">
      <header class="topbar"><div class="brand"><img class="brand-logo" src="/inkriver-logo.png" alt=""><div><strong>InkRiver</strong><small>All your feeds. One flow.</small></div></div><nav class="main-navigation" aria-label="Navigation principale"><button data-action="show-articles" aria-current="${this.mainView === "articles" ? "page" : "false"}" class="${this.mainView === "articles" ? "active" : ""}">Articles</button><button data-action="subscriptions" aria-current="${this.mainView === "feeds" ? "page" : "false"}" class="${this.mainView === "feeds" ? "active" : ""}">Abonnements</button></nav><div class="top-actions"><button class="primary" data-action="refresh" ${this.refreshing ? "disabled" : ""}>${this.refreshing ? "Actualisation…" : "Actualiser"}</button></div></header>
      <div class="banners">${this.error ? `<div class="banner error" role="alert">${escapeHtml(this.error)}</div>` : ""}${this.notice ? `<div class="banner notice" role="status">${escapeHtml(this.notice)}</div>` : ""}</div>
      <main>${this.mainView === "articles" ? `<aside class="timeline" aria-label="Articles">${this.renderArticleViews()}${this.renderArticleList()}</aside><section class="reader">${this.renderReader()}</section>` : this.renderFeedManagement()}</main>
      ${this.renderAddSubscription()}
    </div>`;

    const timeline = this.root.querySelector<HTMLElement>(".timeline");
    if (timeline && timelineScrollTop !== undefined) {
      timeline.scrollTop = timelineScrollTop;
    }

    const frame = this.root.querySelector<HTMLIFrameElement>(".article-content");
    if (frame && this.selected?.content) {
      const nonce = globalThis.crypto.randomUUID();
      frame.srcdoc = buildArticleDocument(this.selected.content, nonce);
    }
    this.bindEvents();
  }

  private bindEvents(): void {
    this.root.querySelectorAll<HTMLElement>('[data-action="article-view"]').forEach((element) => {
      element.addEventListener("click", () =>
        this.showArticleView(element.dataset.articleView as ArticleView),
      );
    });
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
        this.mainView = "feeds";
        this.render();
      });
    });
    this.root.querySelector<HTMLElement>('[data-action="show-articles"]')?.addEventListener("click", () => {
      this.mainView = "articles";
      this.render();
      this.scrollSelectedArticleIntoView();
    });
    this.root.querySelectorAll<HTMLElement>('[data-action="add-subscription"]').forEach((element) => {
      element.addEventListener("click", () => {
        this.addSubscriptionOpen = true;
        this.render();
      });
    });
    this.root.querySelector<HTMLElement>('[data-action="close-add-subscription"]')?.addEventListener("click", () => {
      this.addSubscriptionOpen = false;
      this.render();
    });
    this.root.querySelector<HTMLElement>('[data-action="refresh"]')?.addEventListener("click", () => void this.refresh());
    this.root.querySelector<HTMLElement>('[data-action="toggle-read"]')?.addEventListener("click", () => void this.toggleReadState());
    this.root.querySelector<HTMLElement>('[data-action="favorite"]')?.addEventListener("click", () => void this.toggleFavorite());
    this.root.querySelector<HTMLElement>('[data-action="open-source"]')?.addEventListener("click", () => void this.openSelectedOriginal());
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
