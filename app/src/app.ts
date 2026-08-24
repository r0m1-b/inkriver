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
type ConfirmAction = (message: string) => boolean;
type ArticleView = "all" | "favorites" | "unread";
type MainView = "articles" | "feeds";
type MobileArticleScreen = "timeline" | "reader";
type NoticeKind = "success" | "error";
type ArchiveActionOrigin = "reader-header" | "reader-mobile" | "timeline";
export type ArticleTextSize = "small" | "medium" | "large";
const ARTICLE_LINK_MESSAGE = "inkriver:article-link";
const ARTICLE_HEIGHT_MESSAGE = "inkriver:article-height";
const ARTICLE_IMAGE_MESSAGE = "inkriver:article-image";
const ARTICLE_IMAGE_FOCUS_MESSAGE = "inkriver:article-image-focus";
const ARTICLE_TEXT_SIZE_MESSAGE = "inkriver:article-text-size";
// Keep ARTICLE_BRIDGE_CSP_HASH synchronized with this exact script. The hash is
// also declared in tauri.conf.json because about:srcdoc inherits the app CSP.
export const ARTICLE_BRIDGE_SCRIPT = `function reportArticleHeight(){var root=document.documentElement;var body=document.body;var height=Math.max(root.scrollHeight,root.offsetHeight,body?body.scrollHeight:0,body?body.offsetHeight:0);window.parent.postMessage({type:"inkriver:article-height",height:height},"*");}function openArticleImage(image){var src=image.currentSrc||image.getAttribute("src");if(!src)return;window.parent.postMessage({type:"inkriver:article-image",src:src,alt:image.getAttribute("alt")||"",imageId:image.getAttribute("data-zoomable-image")||""},"*");}document.addEventListener("click",function(event){var target=event.target;var image=target&&target.closest?target.closest("img[data-zoomable-image]"):null;if(image){event.preventDefault();event.stopPropagation();openArticleImage(image);return;}var link=target&&target.closest?target.closest("a[data-external-href],a[data-internal-fragment]"):null;if(!link)return;event.preventDefault();var href=link.getAttribute("data-external-href");if(href){window.parent.postMessage({type:"inkriver:article-link",href:href},"*");return;}var fragment=link.getAttribute("data-internal-fragment");if(!fragment)return;var destination=document.getElementById(fragment)||document.getElementsByName(fragment)[0];if(destination)destination.scrollIntoView({block:"start",inline:"nearest"});},true);document.addEventListener("keydown",function(event){if(event.key!=="Enter"&&event.key!==" ")return;var target=event.target;var image=target&&target.closest?target.closest("img[data-zoomable-image]"):null;if(!image)return;event.preventDefault();openArticleImage(image);},true);window.addEventListener("message",function(event){var message=event.data;if(!message)return;if(message.type==="inkriver:article-image-focus"&&typeof message.imageId==="string"){var image=document.querySelector('img[data-zoomable-image="'+CSS.escape(message.imageId)+'"]');if(image)image.focus();return;}if(message.type==="inkriver:article-text-size"&&[16,18,22].includes(message.fontSize)){document.documentElement.style.setProperty("--article-font-size",message.fontSize+"px");reportArticleHeight();}});window.addEventListener("load",reportArticleHeight);new ResizeObserver(reportArticleHeight).observe(document.documentElement);reportArticleHeight();`;
export const ARTICLE_BRIDGE_CSP_HASH =
  "sha256-X9uP4ATgOR+t0SnpN8j14L1YqawxW54jpnjiw5UzjEQ=";
const ARTICLE_TEXT_SIZE_STORAGE_KEY = "inkriver.articleTextSize";
const ARTICLE_TEXT_SIZES: readonly ArticleTextSize[] = ["small", "medium", "large"];
const ARTICLE_TEXT_SIZE_CONFIG: Record<ArticleTextSize, { label: string; pixels: number }> = {
  small: { label: "Petit", pixels: 16 },
  medium: { label: "Moyen", pixels: 18 },
  large: { label: "Grand", pixels: 22 },
};
const MAX_ARTICLE_FRAME_HEIGHT = 10_000_000;
const NOTICE_TIMEOUT_MS = 8_000;
const NOTICE_FADE_MS = 180;
const IMAGE_ZOOM_FADE_MS = 180;
const TOP_BUTTON_SHOW_RATIO = 1;
const PULL_REFRESH_THRESHOLD = 72;
const PULL_REFRESH_MAX_DISTANCE = 96;
const TOP_BUTTON_HIDE_RATIO = 0.75;

type PreferenceStorage = Pick<Storage, "getItem" | "setItem">;

function isArticleTextSize(value: unknown): value is ArticleTextSize {
  return ARTICLE_TEXT_SIZES.includes(value as ArticleTextSize);
}

export function readArticleTextSize(
  storage: Pick<PreferenceStorage, "getItem"> | null,
): ArticleTextSize {
  try {
    const value = storage?.getItem(ARTICLE_TEXT_SIZE_STORAGE_KEY);
    return isArticleTextSize(value) ? value : "medium";
  } catch {
    return "medium";
  }
}

export function writeArticleTextSize(
  storage: Pick<PreferenceStorage, "setItem"> | null,
  size: ArticleTextSize,
): void {
  try {
    storage?.setItem(ARTICLE_TEXT_SIZE_STORAGE_KEY, size);
  } catch {
    // The preference remains available for the current application session.
  }
}

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

function renderSourceBadge(platform: Platform, logoDataUrl: string | null = null): string {
  const safeLogo = platform === "other" && logoDataUrl?.startsWith("data:image/png;base64,")
    ? logoDataUrl
    : null;
  const icon = safeLogo
    ? `<img class="source-icon website-icon" data-feed-logo src="${escapeHtml(safeLogo)}" alt="">`
    : sourceIcon(platform);
  const websiteClass = safeLogo ? " website" : "";
  return `<span class="source-identity"><span class="source-logo ${platform}${websiteClass}">${icon}</span></span>`;
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
  return Boolean(
    articleSourceHost(article.url) &&
      ["excerpt", "missing", "unknown"].includes(article.contentKind),
  );
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

export function resolveArticleImageUrl(
  rawUrl: string,
  articleUrl?: string | null,
): string {
  const value = rawUrl.trim();
  if (/^data:image\/[a-z0-9.+-]+[;,]/i.test(value)) return value;
  let url: URL;
  try {
    url = articleUrl ? new URL(value, articleUrl) : new URL(value);
  } catch {
    throw new Error(`Image invalide : ${rawUrl}`);
  }
  if (url.protocol !== "https:" || url.username || url.password) {
    throw new Error("Seules les images HTTPS ou data: peuvent être agrandies.");
  }
  return url.toString();
}

export function prepareArticleContent(content: string): string {
  const document = new DOMParser().parseFromString(content, "text/html");
  document.querySelectorAll<HTMLImageElement>("img[src]").forEach((image, index) => {
    image.dataset.zoomableImage = String(index);
    image.tabIndex = 0;
    image.setAttribute("role", "button");
    image.setAttribute(
      "aria-label",
      image.alt ? `Agrandir l’image : ${image.alt}` : "Agrandir l’image",
    );
  });
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

export function buildArticleDocument(
  content: string,
  textSize: ArticleTextSize = "medium",
): string {
  const preparedContent = prepareArticleContent(content);
  const fontSize = ARTICLE_TEXT_SIZE_CONFIG[textSize].pixels;
  return `<!doctype html><html style="--article-font-size:${fontSize}px"><head><meta charset="utf-8"><meta name="color-scheme" content="light dark"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src https: data:; style-src 'unsafe-inline'; script-src '${ARTICLE_BRIDGE_CSP_HASH}'; base-uri 'none'; form-action 'none'"><style>html,body{overflow:hidden}body{font-family:Georgia,serif;font-size:var(--article-font-size);line-height:1.75;max-width:720px;margin:0 auto;padding:8px 32px 64px;color:#292621;background:transparent}img{max-width:100%;height:auto}img[data-zoomable-image]{cursor:zoom-in}img[data-zoomable-image]:focus-visible{outline:3px solid #a84d2f;outline-offset:3px}a{color:#a84d2f}pre{white-space:pre-wrap}@media(max-width:720px){body{padding:8px 18px 48px}}@media(prefers-color-scheme:dark){body{color:#e8e1d8}}</style><script>${ARTICLE_BRIDGE_SCRIPT}</script></head><body>${preparedContent}</body></html>`;
}

function favoriteIcon(isFavorite: boolean): string {
  return `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="m12 2.75 2.86 5.8 6.4.93-4.63 4.51 1.09 6.37L12 17.35l-5.72 3.01 1.09-6.37-4.63-4.51 6.4-.93L12 2.75Z" ${isFavorite ? 'fill="currentColor"' : 'fill="none" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round"'}/></svg>`;
}

function readIcon(isRead: boolean): string {
  return isRead
    ? '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M3 10 12 3l9 7v10H3V10Z" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round"/><path d="m3 10 9 7 9-7" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round"/></svg>'
    : '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M3 6h18v13H3V6Z" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round"/><path d="m3 7 9 7 9-7" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round"/></svg>';
}

function refreshIcon(): string {
  return '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M21 12a9 9 0 0 1-15.22 6.5L3 16m0 0v5m0-5h5M3 12A9 9 0 0 1 18.22 5.5L21 8m0 0V3m0 5h-5" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"/></svg>';
}

function archiveIcon(): string {
  return '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 7.5h16v12H4v-12Zm-1-3h18v3H3v-3Zm6 7h6" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/></svg>';
}

function topIcon(): string {
  return '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="m6 11 6-6 6 6M12 5v14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>';
}

function backIcon(): string {
  return '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="m15 18-6-6 6-6" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>';
}

function externalLinkIcon(): string {
  return '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M14 5h5v5M19 5l-9 9M18 13v6H5V6h6" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/></svg>';
}

function textZoomIcon(sign: "plus" | "minus"): string {
  const signPath = sign === "plus" ? "M10.5 7v5M8 9.5h5" : "M8 9.5h5";
  return `<svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="10" cy="10" r="6" fill="none" stroke="currentColor" stroke-width="1.8"/><path d="m14.5 14.5 5 5" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"/><path d="${signPath}" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"/></svg>`;
}

export class InkRiverApp {
  private articles: ArticleSummary[] = [];
  private feeds: Feed[] = [];
  private selected: ArticleDetail | null = null;
  private articleView: ArticleView = "all";
  private articleTextSize: ArticleTextSize = "medium";
  private pendingTextSizeProgress: number | null = null;
  private readonly preferenceStorage: PreferenceStorage | null;
  private mainView: MainView = "articles";
  private mobileArticleScreen: MobileArticleScreen = "timeline";
  private loading = true;
  private refreshing = false;
  private refreshingFeedId: string | null = null;
  private readonly updatingReadArticleIds = new Set<string>();
  private readonly updatingFavoriteArticleIds = new Set<string>();
  private addSubscriptionOpen = false;
  private deletingFeedId: string | null = null;
  private archivingArticleId: string | null = null;
  private archiveConfirmationArticleId: string | null = null;
  private archiveActionOrigin: ArchiveActionOrigin = "reader-header";
  private zoomedImage: { url: string; alt: string; imageId: string } | null = null;
  private imageZoomAppearing = false;
  private imageZoomDismissing = false;
  private imageZoomDismissTimer: ReturnType<typeof setTimeout> | null = null;
  private error: string | null = null;
  private notice: string | null = null;
  private noticeKind: NoticeKind = "success";
  private noticeTimer: ReturnType<typeof setTimeout> | null = null;
  private noticeTimerStartedAt = 0;
  private noticeRemainingMs = NOTICE_TIMEOUT_MS;
  private noticeHovered = false;
  private noticeFocused = false;
  private noticeAppearing = false;
  private noticeDismissing = false;
  private noticeDismissTimer: ReturnType<typeof setTimeout> | null = null;

  constructor(
    private readonly root: HTMLElement,
    private readonly api: InkRiverApi,
    private readonly openOriginal: OpenOriginal,
    private readonly confirmAction: ConfirmAction,
  ) {
    let preferenceStorage: PreferenceStorage | null = null;
    try {
      preferenceStorage = this.root.ownerDocument.defaultView?.localStorage ?? null;
    } catch {
      // Access can be denied for restricted WebView origins.
    }
    this.preferenceStorage = preferenceStorage;
    this.articleTextSize = readArticleTextSize(this.preferenceStorage);
    this.root.ownerDocument.defaultView?.addEventListener("message", (event) => {
      const frame = this.root.querySelector<HTMLIFrameElement>(".article-content");
      if (!frame || event.source !== frame.contentWindow) return;
      const message = event.data as {
        type?: unknown;
        href?: unknown;
        height?: unknown;
        src?: unknown;
        alt?: unknown;
        imageId?: unknown;
      } | null;
      if (!message) {
        return;
      }
      if (message.type === ARTICLE_HEIGHT_MESSAGE && typeof message.height === "number") {
        const height = Math.ceil(message.height);
        if (Number.isFinite(height) && height > 0 && height <= MAX_ARTICLE_FRAME_HEIGHT) {
          frame.style.height = `${height}px`;
          this.restoreTextSizeProgress();
        }
        return;
      }
      if (message.type === ARTICLE_LINK_MESSAGE && typeof message.href === "string") {
        void this.openExternalUrl(message.href, this.selected?.url);
        return;
      }
      if (
        message.type === ARTICLE_IMAGE_MESSAGE &&
        typeof message.src === "string" &&
        typeof message.imageId === "string"
      ) {
        this.openArticleImage(
          message.src,
          typeof message.alt === "string" ? message.alt : "",
          message.imageId,
        );
      }
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
    if (this.selected?.id === articleId) {
      this.mobileArticleScreen = "reader";
      this.render();
      return;
    }
    this.discardImageZoom();
    this.pendingTextSizeProgress = null;
    this.error = null;
    try {
      this.selected = await this.api.getArticle(articleId);
      if (!this.selected.isRead) {
        await this.setArticleReadState(articleId, true);
      }
      this.mobileArticleScreen = "reader";
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

  private textSizeControlState(): {
    label: string;
    canDecrease: boolean;
    canIncrease: boolean;
    decreaseLabel: string;
    increaseLabel: string;
  } {
    const index = ARTICLE_TEXT_SIZES.indexOf(this.articleTextSize);
    const previous = ARTICLE_TEXT_SIZES[index - 1];
    const next = ARTICLE_TEXT_SIZES[index + 1];
    return {
      label: ARTICLE_TEXT_SIZE_CONFIG[this.articleTextSize].label,
      canDecrease: previous !== undefined,
      canIncrease: next !== undefined,
      decreaseLabel: previous
        ? `Réduire le texte : ${ARTICLE_TEXT_SIZE_CONFIG[previous].label}`
        : "Taille de texte minimale",
      increaseLabel: next
        ? `Agrandir le texte : ${ARTICLE_TEXT_SIZE_CONFIG[next].label}`
        : "Taille de texte maximale",
    };
  }

  private changeArticleTextSize(offset: -1 | 1): void {
    const currentIndex = ARTICLE_TEXT_SIZES.indexOf(this.articleTextSize);
    const nextSize = ARTICLE_TEXT_SIZES[currentIndex + offset];
    if (!nextSize) return;

    const reader = this.root.querySelector<HTMLElement>(".reader");
    const maxScroll = reader
      ? Math.max(0, reader.scrollHeight - reader.clientHeight)
      : 0;
    this.pendingTextSizeProgress = reader && maxScroll > 0
      ? Math.min(1, Math.max(0, reader.scrollTop / maxScroll))
      : 0;
    this.articleTextSize = nextSize;
    writeArticleTextSize(this.preferenceStorage, nextSize);
    this.syncTextSizeControls();

    const frame = this.root.querySelector<HTMLIFrameElement>(".article-content");
    if (!frame?.contentWindow) {
      this.pendingTextSizeProgress = null;
      return;
    }
    frame.contentWindow.postMessage(
      {
        type: ARTICLE_TEXT_SIZE_MESSAGE,
        fontSize: ARTICLE_TEXT_SIZE_CONFIG[nextSize].pixels,
      },
      "*",
    );
  }

  private syncTextSizeControls(): void {
    const state = this.textSizeControlState();
    this.root.querySelectorAll<HTMLElement>("[data-text-size-label]").forEach((label) => {
      label.textContent = state.label;
    });
    this.root.querySelectorAll<HTMLButtonElement>('[data-action="decrease-text-size"]').forEach((button) => {
      button.disabled = !state.canDecrease;
      button.title = state.decreaseLabel;
      button.setAttribute("aria-label", state.decreaseLabel);
    });
    this.root.querySelectorAll<HTMLButtonElement>('[data-action="increase-text-size"]').forEach((button) => {
      button.disabled = !state.canIncrease;
      button.title = state.increaseLabel;
      button.setAttribute("aria-label", state.increaseLabel);
    });
  }

  private restoreTextSizeProgress(): void {
    if (this.pendingTextSizeProgress === null) return;
    const reader = this.root.querySelector<HTMLElement>(".reader");
    if (reader) {
      const maxScroll = Math.max(0, reader.scrollHeight - reader.clientHeight);
      reader.scrollTop = this.pendingTextSizeProgress * maxScroll;
      this.updateReaderTopButton(reader);
    }
    this.pendingTextSizeProgress = null;
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

  private cancelNoticeTimer(): void {
    if (this.noticeTimer === null) return;
    clearTimeout(this.noticeTimer);
    this.noticeTimer = null;
  }

  private cancelNoticeDismissTimer(): void {
    if (this.noticeDismissTimer === null) return;
    clearTimeout(this.noticeDismissTimer);
    this.noticeDismissTimer = null;
  }

  private clearNotice(): void {
    this.cancelNoticeTimer();
    this.cancelNoticeDismissTimer();
    this.notice = null;
    this.noticeKind = "success";
    this.noticeRemainingMs = NOTICE_TIMEOUT_MS;
    this.noticeHovered = false;
    this.noticeFocused = false;
    this.noticeAppearing = false;
    this.noticeDismissing = false;
  }

  private showNotice(message: string, kind: NoticeKind = "success"): void {
    this.clearNotice();
    this.notice = message;
    this.noticeKind = kind;
    this.noticeAppearing = true;
    this.resumeNoticeTimer();
  }

  private pauseNoticeTimer(): void {
    if (this.noticeTimer === null) return;
    clearTimeout(this.noticeTimer);
    this.noticeTimer = null;
    this.noticeRemainingMs = Math.max(
      0,
      this.noticeRemainingMs - (Date.now() - this.noticeTimerStartedAt),
    );
  }

  private resumeNoticeTimer(): void {
    if (
      !this.notice ||
      this.noticeTimer !== null ||
      this.noticeHovered ||
      this.noticeFocused ||
      this.noticeDismissing
    ) return;
    if (this.noticeRemainingMs <= 0) {
      this.dismissNotice();
      return;
    }
    this.noticeTimerStartedAt = Date.now();
    this.noticeTimer = setTimeout(() => {
      this.noticeTimer = null;
      this.noticeRemainingMs = 0;
      this.dismissNotice();
    }, this.noticeRemainingMs);
  }

  private dismissNotice(): void {
    if (!this.notice || this.noticeDismissing) return;
    this.cancelNoticeTimer();
    this.noticeHovered = false;
    this.noticeFocused = false;
    this.noticeDismissing = true;
    this.render();
    this.noticeDismissTimer = setTimeout(() => {
      this.clearNotice();
      this.render();
    }, NOTICE_FADE_MS);
  }

  private discardImageZoom(): void {
    if (this.imageZoomDismissTimer !== null) {
      clearTimeout(this.imageZoomDismissTimer);
      this.imageZoomDismissTimer = null;
    }
    this.zoomedImage = null;
    this.imageZoomAppearing = false;
    this.imageZoomDismissing = false;
  }

  private openArticleImage(rawUrl: string, alt: string, imageId: string): void {
    let url: string;
    try {
      url = resolveArticleImageUrl(rawUrl, this.selected?.url);
    } catch {
      return;
    }
    this.discardImageZoom();
    this.zoomedImage = { url, alt, imageId };
    this.imageZoomAppearing = true;
    this.render();
  }

  private closeImageZoom(): void {
    if (!this.zoomedImage || this.imageZoomDismissing) return;
    this.imageZoomDismissing = true;
    this.render();
    this.imageZoomDismissTimer = setTimeout(() => {
      const imageId = this.zoomedImage?.imageId;
      this.discardImageZoom();
      this.render();
      if (imageId !== undefined) this.restoreArticleImageFocus(imageId);
    }, IMAGE_ZOOM_FADE_MS);
  }

  private restoreArticleImageFocus(imageId: string): void {
    const frame = this.root.querySelector<HTMLIFrameElement>(".article-content");
    if (!frame) return;
    const restore = () =>
      frame.contentWindow?.postMessage(
        { type: ARTICLE_IMAGE_FOCUS_MESSAGE, imageId },
        "*",
      );
    restore();
    frame.addEventListener("load", restore, { once: true });
  }

  private updateReaderTopButton(reader: HTMLElement): void {
    const button = this.root.querySelector<HTMLButtonElement>(
      '[data-action="reader-top"]',
    );
    if (!button) return;
    const isVisible = button.classList.contains("visible");
    const thresholdRatio = isVisible
      ? TOP_BUTTON_HIDE_RATIO
      : TOP_BUTTON_SHOW_RATIO;
    const shouldBeVisible =
      reader.clientHeight > 0 &&
      reader.scrollTop > reader.clientHeight * thresholdRatio;
    if (isVisible === shouldBeVisible) return;
    button.classList.toggle("visible", shouldBeVisible);
    button.setAttribute("aria-hidden", String(!shouldBeVisible));
    button.tabIndex = shouldBeVisible ? 0 : -1;
    if (!shouldBeVisible && this.root.ownerDocument.activeElement === button) {
      button.blur();
    }
  }

  private scrollReaderToTop(): void {
    const reader = this.root.querySelector<HTMLElement>(".reader");
    const button = this.root.querySelector<HTMLButtonElement>(
      '[data-action="reader-top"]',
    );
    if (!reader || !button) return;
    button.blur();
    const reduceMotion =
      this.root.ownerDocument.defaultView
        ?.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false;
    reader.scrollTo({ top: 0, behavior: reduceMotion ? "auto" : "smooth" });
  }

  private async refresh(): Promise<void> {
    if (this.refreshing) return;
    this.refreshing = true;
    this.refreshingFeedId = null;
    this.error = null;
    this.clearNotice();
    this.render();
    try {
      const report = await this.api.refreshFeeds();
      [this.articles, this.feeds] = await Promise.all([
        this.api.listArticles(),
        this.api.listFeeds(),
      ]);
      this.showNotice(this.refreshNotice(report));
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

  private async refreshFeed(feedId: string): Promise<void> {
    const feed = this.feeds.find((candidate) => candidate.id === feedId);
    if (!feed || !feed.isActive || this.refreshing) return;
    const feedName = feed.title ?? feed.url;
    this.refreshing = true;
    this.refreshingFeedId = feedId;
    this.error = null;
    this.clearNotice();
    this.render();
    try {
      const report = await this.api.refreshFeed(feedId);
      [this.articles, this.feeds] = await Promise.all([
        this.api.listArticles(),
        this.api.listFeeds(),
      ]);
      if (this.selected) {
        this.selected = await this.api.getArticle(this.selected.id).catch(() => null);
      }
      const collectionError = report.errors[0];
      if (collectionError) {
        this.showNotice(
          `Échec de l’actualisation de « ${feedName} » — ${collectionError.stage} : ${collectionError.message}`,
          "error",
        );
      } else {
        this.showNotice(
          `« ${feedName} » actualisé : ${this.refreshResult(report)}.`,
        );
      }
    } catch (error) {
      this.showNotice(
        `Impossible d’actualiser « ${feedName} » : ${errorMessage(error)}`,
        "error",
      );
    } finally {
      this.refreshing = false;
      this.refreshingFeedId = null;
      this.render();
    }
  }

  private refreshResult(report: RefreshReport): string {
    return `${report.insertedArticles} nouveau(x), ${report.updatedArticles} actualisé(s), ${report.extractedArticles} extrait(s), ${report.extractionFailedArticles} extraction(s) en échec, ${report.extractionSkippedArticles} différée(s), ${report.autoArchivedArticles} ancien(s) supprimé(s) automatiquement`;
  }

  private refreshNotice(report: RefreshReport): string {
    const result = this.refreshResult(report);
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
      this.showNotice("Abonnement ajouté. Utilisez le bouton d’actualisation de sa carte pour télécharger ses articles.");
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
    const confirmed = this.confirmAction(
      `Supprimer l’abonnement « ${feed.url} » ? Ses articles, favoris et états de lecture seront définitivement supprimés.`,
    );
    if (!confirmed) return;

    this.deletingFeedId = feedId;
    this.error = null;
    this.clearNotice();
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
      this.showNotice(`Abonnement supprimé avec ${result.deletedArticles} ${label}.`);
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

  private requestArchiveSelectedArticle(origin: ArchiveActionOrigin = "reader-header"): void {
    if (!this.selected) return;
    this.requestArchiveArticle(this.selected.id, origin);
  }

  private requestArchiveArticle(
    articleId: string,
    origin: ArchiveActionOrigin,
  ): void {
    if (!this.articles.some((article) => article.id === articleId)) return;
    this.archiveActionOrigin = origin;
    this.archiveConfirmationArticleId = articleId;
    this.render();
  }

  private cancelArchiveSelectedArticle(): void {
    const articleId = this.archiveConfirmationArticleId;
    const origin = this.archiveActionOrigin;
    this.archiveConfirmationArticleId = null;
    this.render();
    if (origin === "timeline") {
      Array.from(
        this.root.querySelectorAll<HTMLElement>(
          '[data-action="timeline-archive"]',
        ),
      )
        .find((element) => element.dataset.articleId === articleId)
        ?.focus();
      return;
    }
    const selector = origin === "reader-mobile"
      ? '.mobile-reader-toolbar [data-action="archive-article"]'
      : '.desktop-reader-actions [data-action="archive-article"]';
    this.root.querySelector<HTMLElement>(selector)?.focus();
  }

  private async confirmArchiveSelectedArticle(): Promise<void> {
    const articleId = this.archiveConfirmationArticleId;
    if (!articleId || !this.articles.some((article) => article.id === articleId)) {
      this.archiveConfirmationArticleId = null;
      this.render();
      return;
    }

    this.archiveConfirmationArticleId = null;
    this.archivingArticleId = articleId;
    this.error = null;
    this.clearNotice();
    this.render();
    try {
      await this.api.archiveArticle(articleId);
      this.articles = this.articles.filter((candidate) => candidate.id !== articleId);
      if (this.selected?.id === articleId) {
        this.selected = null;
        this.mobileArticleScreen = "timeline";
      }
      this.showNotice("Article archivé.");
    } catch (error) {
      this.error = errorMessage(error);
    } finally {
      this.archivingArticleId = null;
      this.render();
    }
  }

  private renderArticleList(): string {
    if (this.loading) return '<div class="state" data-testid="loading">Chargement du cache…</div>';
    if (this.articles.length === 0) {
      return '<div class="state" data-testid="empty">Aucun article enregistré.<button class="text-button" data-action="add-subscription">Ajouter un abonnement</button></div>';
    }
    const visibleArticles = this.articleView === "favorites"
      ? this.articles.filter((article) => article.isFavorite)
      : this.articleView === "unread"
        ? this.articles.filter((article) => !article.isRead)
        : this.articles;
    if (visibleArticles.length === 0) {
      if (this.articleView === "unread") {
        return '<div class="state" data-testid="unread-empty"><strong>Aucun article non lu.</strong><span>Sélectionnez « Tous » pour retrouver les articles lus.</span></div>';
      }
      return '<div class="state" data-testid="favorites-empty"><strong>Aucun article favori.</strong><span>Utilisez l’étoile pour retrouver un article ici.</span></div>';
    }
    return visibleArticles
      .map((article) => {
        const title = article.title ?? "Sans titre";
        const favoriteAction = article.isFavorite ? "Retirer des favoris" : "Ajouter aux favoris";
        const readAction = article.isRead ? "Marquer comme non lu" : "Marquer comme lu";
        const favoritePending = this.updatingFavoriteArticleIds.has(article.id);
        const readPending = this.updatingReadArticleIds.has(article.id);
        const archivePending = this.archivingArticleId === article.id;

        return `<article class="article-row ${article.isRead ? "read" : "unread"} ${this.selected?.id === article.id ? "selected" : ""}" data-article-row-id="${escapeHtml(article.id)}">
          <button type="button" class="article-select" data-action="select-article" data-article-id="${escapeHtml(article.id)}">
          <span class="article-list-logo">${renderSourceBadge(article.source, this.feedLogo(article.feedId))}</span>
          <span class="article-list-copy">
            <span class="row-top"><span class="byline">${escapeHtml(article.author ?? "Auteur inconnu")}</span><time>${displayDate(article.publishedAt)}</time></span>
            <strong title="${escapeHtml(title)}">${escapeHtml(title)}</strong>
          </span>
          </button>
          <span class="article-row-actions">
            <button type="button" class="article-icon-button favorite ${article.isFavorite ? "active" : ""}" data-action="timeline-favorite" data-state-article-id="${escapeHtml(article.id)}" aria-label="${escapeHtml(`${favoriteAction} : ${title}`)}" title="${favoriteAction}" aria-pressed="${article.isFavorite}" aria-busy="${favoritePending}" ${favoritePending || archivePending ? "disabled" : ""}>${favoriteIcon(article.isFavorite)}</button>
            <button type="button" class="article-icon-button read-state-icon ${article.isRead ? "active" : ""}" data-action="timeline-read" data-state-article-id="${escapeHtml(article.id)}" aria-label="${escapeHtml(`${readAction} : ${title}`)}" title="${readAction}" aria-pressed="${article.isRead}" aria-busy="${readPending}" ${readPending || archivePending ? "disabled" : ""}>${readIcon(article.isRead)}</button>
            <button type="button" class="article-icon-button danger" data-action="timeline-archive" data-article-id="${escapeHtml(article.id)}" title="Archiver l’article" aria-label="${escapeHtml(`Archiver l’article : ${title}`)}" aria-busy="${archivePending}" ${archivePending ? "disabled" : ""}>${archiveIcon()}</button>
          </span>
        </article>`;
      })
      .join("");
  }

  private renderArticleViews(): string {
    const favoriteCount = this.articles.filter((article) => article.isFavorite).length;
    const unreadCount = this.articles.filter((article) => !article.isRead).length;
    return `<div class="timeline-tabs">
      <nav class="timeline-view-tabs" aria-label="Vues des articles" role="tablist">
        <button type="button" class="timeline-tab ${this.articleView === "all" ? "active" : ""}" data-action="article-view" data-article-view="all" role="tab" aria-selected="${this.articleView === "all"}">Tous</button>
        <button type="button" class="timeline-tab ${this.articleView === "favorites" ? "active" : ""}" data-action="article-view" data-article-view="favorites" role="tab" aria-selected="${this.articleView === "favorites"}">Favoris <span class="tab-count">${favoriteCount}</span></button>
        <button type="button" class="timeline-tab ${this.articleView === "unread" ? "active" : ""}" data-action="article-view" data-article-view="unread" role="tab" aria-selected="${this.articleView === "unread"}">Non lus <span class="tab-count">${unreadCount}</span></button>
      </nav>
    </div>`;
  }

  private renderPullRefresh(): string {
    const label = this.refreshing ? "Actualisation en cours…" : "Tirez pour actualiser";
    return `<div class="pull-refresh${this.refreshing ? " refreshing" : ""}" data-pull-refresh aria-hidden="${!this.refreshing}"><span class="pull-refresh-icon">${refreshIcon()}</span><span data-pull-refresh-label>${label}</span></div>`;
  }

  private feedLogo(feedId: string): string | null {
    return this.feeds.find((feed) => feed.id === feedId)?.logoDataUrl ?? null;
  }

  private renderReader(): string {
    if (!this.selected) {
      return '<div class="reader-placeholder"><img class="reader-placeholder-logo" src="/inkriver-wordmark.png" alt="InkRiver"><p>Sélectionnez un article dans la chronologie.</p></div>';
    }
    const article = this.selected;
    const sourceHost = articleSourceHost(article.url);
    const sourceLink = sourceHost && article.url
      ? `<div class="article-source">${renderSourceBadge(article.source, this.feedLogo(article.feedId))} <button type="button" class="article-source-link" data-action="open-source" title="${escapeHtml(article.url)}" aria-label="${escapeHtml(`Ouvrir la source ${sourceHost} dans le navigateur`)}">${escapeHtml(sourceHost)} ↗</button></div>`
      : `<div class="article-source">${article.url ? "Source non prise en charge" : "Source indisponible"}</div>`;
    const content = article.content
      ? '<iframe class="article-content" title="Contenu de l’article" sandbox="allow-scripts" scrolling="no"></iframe>'
      : '<p class="missing-content">Le flux ne fournit pas de contenu pour cet article.</p>';
    return `<article class="reader-article">
      <header>
      <h1>${escapeHtml(article.title ?? "Sans titre")}</h1>
      <p>${escapeHtml(article.author ?? "Auteur inconnu")} · ${displayDate(article.publishedAt)}</p>
      ${sourceLink}
      <!-- <div class="read-state" data-testid="read-state">État : <strong>${article.isRead ? "Lu" : "Non lu"}</strong></div> -->
      ${this.renderReaderActions(article, false)}</header>
      ${content}
      ${this.renderReaderFooter(article)}
    </article>`;
  }

  private renderMobileReaderToolbar(): string {
    if (!this.selected) return "";
    return `<nav class="mobile-reader-toolbar" aria-label="Navigation et actions du lecteur"><button type="button" class="mobile-reader-back" data-action="mobile-reader-back" title="Retour aux articles" aria-label="Retour aux articles">${backIcon()}</button>${this.renderReaderActions(this.selected, true)}</nav>`;
  }

  private renderReaderActions(article: ArticleDetail, mobile: boolean): string {
    const readPending = this.updatingReadArticleIds.has(article.id);
    const favoritePending = this.updatingFavoriteArticleIds.has(article.id);
    const archivePending = this.archivingArticleId === article.id;
    const readAction = article.isRead ? "Marquer comme non lu" : "Marquer comme lu";
    const favoriteAction = article.isFavorite ? "Retirer des favoris" : "Ajouter aux favoris";
    const originalButton = canOpenOriginal(article)
      ? mobile
        ? `<button type="button" class="reader-icon-button primary" data-action="open-original" title="Lire l’original" aria-label="Lire l’original">${externalLinkIcon()}</button>`
        : '<button class="primary" data-action="open-original">Lire l’original ↗</button>'
      : "";
    const modeClass = mobile ? "mobile-reader-actions" : "desktop-reader-actions";
    return `<div class="reader-actions ${modeClass}"><button type="button" class="reader-icon-button read-state-icon ${article.isRead ? "active" : ""}" data-action="toggle-read" title="${readAction}" aria-label="${readAction}" aria-pressed="${article.isRead}" aria-busy="${readPending}" ${readPending || archivePending ? "disabled" : ""}>${readIcon(article.isRead)}</button><button type="button" class="reader-icon-button favorite ${article.isFavorite ? "active" : ""}" data-action="favorite" title="${favoriteAction}" aria-label="${favoriteAction}" aria-pressed="${article.isFavorite}" aria-busy="${favoritePending}" ${favoritePending || archivePending ? "disabled" : ""}>${favoriteIcon(article.isFavorite)}</button><button type="button" class="reader-icon-button danger" data-action="archive-article" title="Archiver l’article" aria-label="Archiver l’article" aria-busy="${archivePending}" ${archivePending ? "disabled" : ""}>${archiveIcon()}</button>${originalButton}${this.renderTextSizeControls(true)}</div>`;
  }

  public handleBackNavigation(): boolean {
    if (this.zoomedImage) {
      this.closeImageZoom();
      return true;
    }
    if (this.archiveConfirmationArticleId) {
      this.cancelArchiveSelectedArticle();
      return true;
    }
    if (this.addSubscriptionOpen) {
      this.addSubscriptionOpen = false;
      this.render();
      return true;
    }
    if (this.mainView === "feeds") {
      this.mainView = "articles";
      this.mobileArticleScreen = "timeline";
      this.render();
      this.scrollSelectedArticleIntoView();
      return true;
    }
    if (this.mobileArticleScreen === "reader") {
      this.mobileArticleScreen = "timeline";
      this.render();
      this.scrollSelectedArticleIntoView();
      return true;
    }
    return false;
  }

  private renderReaderFooter(article: ArticleDetail): string {
    const sourceAvailable = articleSourceHost(article.url) !== null && article.url !== null;
    const sourceAction = sourceAvailable
      ? `Ouvrir le lien : ${article.url}`
      : "Lien source indisponible";
    return `<footer class="reader-footer" aria-label="Actions de fin d’article">
      <button type="button" class="reader-footer-button" data-action="open-source" title="${escapeHtml(sourceAction)}" aria-label="${escapeHtml(sourceAction)}" ${sourceAvailable ? "" : "disabled"}>${externalLinkIcon()}</button>
    </footer>`;
  }

  private renderTextSizeControls(announceChanges: boolean): string {
    const state = this.textSizeControlState();
    return `<div class="text-size-controls" role="group" aria-label="Taille du texte de l’article">
      <button type="button" class="text-size-button" data-action="decrease-text-size" title="${state.decreaseLabel}" aria-label="${state.decreaseLabel}" ${state.canDecrease ? "" : "disabled"}>${textZoomIcon("minus")}</button>
      <button type="button" class="text-size-button" data-action="increase-text-size" title="${state.increaseLabel}" aria-label="${state.increaseLabel}" ${state.canIncrease ? "" : "disabled"}>${textZoomIcon("plus")}</button>
    </div>`;
  }

  private renderReaderTopButton(): string {
    if (!this.selected) return "";
    return `<button type="button" class="reader-top-button" data-action="reader-top" title="Revenir en haut" aria-label="Revenir en haut" aria-hidden="true" tabindex="-1">${topIcon()}</button>`;
  }

  private renderImageZoom(): string {
    if (!this.selected || !this.zoomedImage) return "";
    const label = this.zoomedImage.alt
      ? `Image agrandie : ${this.zoomedImage.alt}`
      : "Image agrandie";
    return `<div class="image-lightbox${this.imageZoomDismissing ? " is-leaving" : this.imageZoomAppearing ? " is-entering" : ""}" data-action="close-image-backdrop">
      <section class="image-lightbox-content" role="dialog" aria-label="${escapeHtml(label)}">
        <img class="image-lightbox-image" src="${escapeHtml(this.zoomedImage.url)}" alt="${escapeHtml(this.zoomedImage.alt)}">
        <button type="button" class="image-lightbox-close" data-action="close-image-zoom" title="Fermer l’image" aria-label="Fermer l’image">×</button>
      </section>
    </div>`;
  }

  private renderFeedManagement(): string {
    const feeds = this.feeds.length
      ? this.feeds
          .map(
            (feed) => `<article class="feed-card ${feed.isActive ? "active" : "inactive"}" data-feed-card-id="${escapeHtml(feed.id)}">
              <header><div>${renderSourceBadge(feed.platform, feed.logoDataUrl)}<span class="feed-status">${feed.isActive ? "Actif" : "Inactif"}</span></div><h2>${escapeHtml(feed.title ?? "Flux non actualisé")}</h2></header>
              <dl>
                <div><dt>URL du flux</dt><dd>${escapeHtml(feed.url)}</dd></div>
                <div><dt>Auteur</dt><dd>${escapeHtml(feed.author ?? "Inconnu")}</dd></div>
                <div class="feed-description"><dt>Description</dt><dd>${escapeHtml(feed.description ?? "Aucune description disponible.")}</dd></div>
                <div><dt>Dernière publication</dt><dd>${displayDateTime(feed.lastPublishedAt)}</dd></div>
                <div><dt>Dernière actualisation réussie</dt><dd>${displayDateTime(feed.lastSuccessAt)}</dd></div>
              </dl>
              ${feed.lastError ? `<section class="feed-error" aria-label="Dernière erreur"><strong>Dernière erreur · ${escapeHtml(feed.lastError.stage)}</strong><time>${displayDateTime(feed.lastError.occurredAt)}</time><p>${escapeHtml(feed.lastError.message)}</p></section>` : ""}
              <footer class="feed-actions"><button type="button" class="feed-refresh-button" data-action="refresh-feed" data-feed-id="${escapeHtml(feed.id)}" title="${feed.isActive ? "Actualiser ce flux" : "Réactivez ce flux pour l’actualiser"}" aria-label="${feed.isActive ? "Actualiser ce flux" : "Réactivez ce flux pour l’actualiser"}" aria-busy="${this.refreshingFeedId === feed.id}" ${!feed.isActive || this.refreshing || this.deletingFeedId !== null ? "disabled" : ""}>${refreshIcon()}</button><button data-action="toggle-feed" data-feed-id="${escapeHtml(feed.id)}" data-next-active="${!feed.isActive}" ${this.refreshing || this.deletingFeedId !== null ? "disabled" : ""}>${feed.isActive ? "Désactiver" : "Réactiver"}</button><button class="danger" data-action="delete-feed" data-feed-id="${escapeHtml(feed.id)}" ${this.refreshing || this.deletingFeedId !== null ? "disabled" : ""}>${this.deletingFeedId === feed.id ? "Suppression…" : "Supprimer"}</button></footer>
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

  private renderArchiveConfirmation(): string {
    const article = this.articles.find(
      (candidate) => candidate.id === this.archiveConfirmationArticleId,
    );
    if (!article) return "";
    const title = escapeHtml(article.title ?? "Sans titre");
    return `<div class="modal-backdrop archive-confirmation-backdrop" data-action="cancel-archive-backdrop">
      <section class="confirmation-dialog archive-confirmation" role="dialog" aria-modal="true" aria-labelledby="archive-confirmation-title" aria-describedby="archive-confirmation-description">
        <header><div class="confirmation-symbol" aria-hidden="true">${archiveIcon()}</div><button type="button" class="icon-button" data-action="cancel-archive" aria-label="Fermer">×</button></header>
        <span class="eyebrow">Archivage</span>
        <h2 id="archive-confirmation-title">Archiver cet article ?</h2>
        <p class="confirmation-subject">« ${title} »</p>
        <p id="archive-confirmation-description">L’article disparaîtra de la chronologie et des favoris. Il ne pourra pas être restauré depuis InkRiver.</p>
        <footer class="confirmation-actions"><button type="button" data-action="cancel-archive">Annuler</button><button type="button" class="danger confirmation-danger" data-action="confirm-archive">${archiveIcon()}<span>Archiver</span></button></footer>
      </section>
    </div>`;
  }

  render(): void {
    const timelineScrollTop =
      this.root.querySelector<HTMLElement>(".timeline")?.scrollTop;
    const feedManagementScrollTop =
      this.root.querySelector<HTMLElement>(".feed-management")?.scrollTop;
    const previousReader = this.root.querySelector<HTMLElement>(".reader");
    const preserveReaderPosition =
      previousReader?.dataset.readerArticleId !== undefined &&
      previousReader.dataset.readerArticleId === this.selected?.id;
    const readerScrollTop = preserveReaderPosition ? previousReader.scrollTop : 0;
    const articleFrameHeight = preserveReaderPosition
      ? this.root.querySelector<HTMLIFrameElement>(".article-content")?.style.height
      : undefined;
    this.root.innerHTML = `<div class="shell">
      <header class="topbar"><div class="brand"><img class="brand-logo" src="/inkriver-logo.png" alt=""><div><strong>InkRiver</strong><small>All your feeds. One flow.</small></div></div><nav class="main-navigation" aria-label="Navigation principale"><button data-action="show-articles" aria-current="${this.mainView === "articles" ? "page" : "false"}" class="${this.mainView === "articles" ? "active" : ""}">Articles</button><button data-action="subscriptions" aria-current="${this.mainView === "feeds" ? "page" : "false"}" class="${this.mainView === "feeds" ? "active" : ""}">Abonnements</button></nav><div class="top-actions"><button type="button" class="primary refresh-button" data-action="refresh" title="Actualiser" aria-label="${this.refreshing ? "Actualisation en cours" : "Actualiser"}" aria-busy="${this.refreshing}" ${this.refreshing ? "disabled" : ""}>${refreshIcon()}</button></div></header>
      <div class="banners">${this.error ? `<div class="banner error" role="alert">${escapeHtml(this.error)}</div>` : ""}${this.notice ? `<div class="banner notice${this.noticeKind === "error" ? " error-notice" : ""}${this.noticeAppearing ? " is-entering" : ""}${this.noticeDismissing ? " is-leaving" : ""}"><span role="${this.noticeKind === "error" ? "alert" : "status"}">${escapeHtml(this.notice)}</span><button type="button" class="banner-dismiss" data-action="dismiss-notice" title="Fermer la notification" aria-label="Fermer la notification">×</button></div>` : ""}</div>
      <main class="main-view ${this.mainView === "articles" ? `articles-view mobile-${this.mobileArticleScreen}` : "feeds-view"}">${this.mainView === "articles" ? `<aside class="timeline" aria-label="Articles">${this.renderPullRefresh()}${this.renderArticleViews()}${this.renderArticleList()}</aside><section class="reader" data-reader-article-id="${escapeHtml(this.selected?.id ?? "")}">${this.renderMobileReaderToolbar()}${this.renderReader()}</section>${this.renderReaderTopButton()}${this.renderImageZoom()}` : this.renderFeedManagement()}</main>
      ${this.renderAddSubscription()}
      ${this.renderArchiveConfirmation()}
    </div>`;
    this.noticeAppearing = false;
    this.imageZoomAppearing = false;

    const timeline = this.root.querySelector<HTMLElement>(".timeline");
    if (timeline && timelineScrollTop !== undefined) {
      timeline.scrollTop = timelineScrollTop;
    }
    const feedManagement =
      this.root.querySelector<HTMLElement>(".feed-management");
    if (feedManagement && feedManagementScrollTop !== undefined) {
      feedManagement.scrollTop = feedManagementScrollTop;
    }

    const frame = this.root.querySelector<HTMLIFrameElement>(".article-content");
    if (frame && this.selected?.content) {
      if (articleFrameHeight) frame.style.height = articleFrameHeight;
      frame.srcdoc = buildArticleDocument(
        this.selected.content,
        this.articleTextSize,
      );
    }
    const reader = this.root.querySelector<HTMLElement>(".reader");
    if (reader && preserveReaderPosition) reader.scrollTop = readerScrollTop;
    this.bindEvents();
    if (reader) this.updateReaderTopButton(reader);
    this.root
      .querySelector<HTMLElement>('.archive-confirmation [data-action="cancel-archive"]')
      ?.focus();
    if (!this.imageZoomDismissing) {
      this.root
        .querySelector<HTMLElement>('[data-action="close-image-zoom"]')
        ?.focus();
    }
  }

  private bindEvents(): void {
    this.root.querySelectorAll<HTMLImageElement>("[data-feed-logo]").forEach((logo) => {
      logo.addEventListener("error", () => {
        const container = logo.closest<HTMLElement>(".source-logo");
        if (!container) return;
        container.classList.remove("website");
        container.innerHTML = sourceIcon("other");
      });
    });
    const noticeBanner = this.root.querySelector<HTMLElement>(".banner.notice");
    this.root
      .querySelector<HTMLElement>('[data-action="dismiss-notice"]')
      ?.addEventListener("click", () => this.dismissNotice());
    noticeBanner?.addEventListener("mouseenter", () => {
      this.noticeHovered = true;
      this.pauseNoticeTimer();
    });
    noticeBanner?.addEventListener("mouseleave", () => {
      this.noticeHovered = false;
      this.resumeNoticeTimer();
    });
    noticeBanner?.addEventListener("focusin", () => {
      this.noticeFocused = true;
      this.pauseNoticeTimer();
    });
    noticeBanner?.addEventListener("focusout", (event) => {
      const nextTarget = event.relatedTarget;
      const view = this.root.ownerDocument.defaultView;
      if (view && nextTarget instanceof view.Node && noticeBanner.contains(nextTarget)) return;
      this.noticeFocused = false;
      this.resumeNoticeTimer();
    });
    const reader = this.root.querySelector<HTMLElement>(".reader");
    reader?.addEventListener(
      "scroll",
      () => this.updateReaderTopButton(reader),
      { passive: true },
    );
    this.bindPullToRefresh();
    this.root
      .querySelector<HTMLElement>('[data-action="reader-top"]')
      ?.addEventListener("click", () => this.scrollReaderToTop());
    this.root
      .querySelector<HTMLElement>('[data-action="mobile-reader-back"]')
      ?.addEventListener("click", () => this.handleBackNavigation());
    this.root.querySelectorAll<HTMLElement>('[data-action="decrease-text-size"]').forEach((element) => {
      element.addEventListener("click", () => this.changeArticleTextSize(-1));
    });
    this.root.querySelectorAll<HTMLElement>('[data-action="increase-text-size"]').forEach((element) => {
      element.addEventListener("click", () => this.changeArticleTextSize(1));
    });
    this.root
      .querySelector<HTMLElement>('[data-action="close-image-zoom"]')
      ?.addEventListener("click", () => this.closeImageZoom());
    const imageLightbox = this.root.querySelector<HTMLElement>(".image-lightbox");
    imageLightbox?.addEventListener("click", (event) => {
      if (event.target === event.currentTarget) this.closeImageZoom();
    });
    imageLightbox?.addEventListener("keydown", (event) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      this.closeImageZoom();
    });
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
    this.root.querySelectorAll<HTMLElement>('[data-action="timeline-archive"]').forEach((element) => {
      element.addEventListener("click", () =>
        this.requestArchiveArticle(element.dataset.articleId!, "timeline"),
      );
    });
    this.root.querySelectorAll<HTMLElement>('[data-action="subscriptions"]').forEach((element) => {
      element.addEventListener("click", () => {
        this.discardImageZoom();
        this.mainView = "feeds";
        this.render();
      });
    });
    this.root.querySelector<HTMLElement>('[data-action="show-articles"]')?.addEventListener("click", () => {
      this.mainView = "articles";
      this.mobileArticleScreen = "timeline";
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
    this.root.querySelectorAll<HTMLElement>('[data-action="refresh-feed"]').forEach((element) => {
      element.addEventListener("click", () => void this.refreshFeed(element.dataset.feedId!));
    });
    this.root.querySelectorAll<HTMLElement>('[data-action="toggle-read"]').forEach((element) => {
      element.addEventListener("click", () => void this.toggleReadState());
    });
    this.root.querySelectorAll<HTMLElement>('[data-action="favorite"]').forEach((element) => {
      element.addEventListener("click", () => void this.toggleFavorite());
    });
    this.root.querySelectorAll<HTMLElement>('[data-action="archive-article"]').forEach((element) => {
      element.addEventListener("click", () =>
        this.requestArchiveSelectedArticle(
          element.closest(".mobile-reader-toolbar") ? "reader-mobile" : "reader-header",
        ),
      );
    });
    this.root.querySelectorAll<HTMLElement>('[data-action="cancel-archive"]').forEach((element) => {
      element.addEventListener("click", () => this.cancelArchiveSelectedArticle());
    });
    this.root.querySelector<HTMLElement>('[data-action="confirm-archive"]')?.addEventListener("click", () => void this.confirmArchiveSelectedArticle());
    this.root.querySelector<HTMLElement>('[data-action="cancel-archive-backdrop"]')?.addEventListener("click", (event) => {
      if (event.target === event.currentTarget) this.cancelArchiveSelectedArticle();
    });
    const archiveDialog = this.root.querySelector<HTMLElement>(".archive-confirmation");
    archiveDialog?.addEventListener("keydown", (event) => {
      if (event.key === "Escape") {
        event.preventDefault();
        this.cancelArchiveSelectedArticle();
        return;
      }
      if (event.key !== "Tab") return;
      const focusable = Array.from(
        archiveDialog.querySelectorAll<HTMLElement>("button:not(:disabled)"),
      );
      if (focusable.length === 0) return;
      const first = focusable[0]!;
      const last = focusable.at(-1)!;
      if (event.shiftKey && this.root.ownerDocument.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && this.root.ownerDocument.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    });
    this.root.querySelectorAll<HTMLElement>('[data-action="open-source"]').forEach((element) => {
      element.addEventListener("click", () => void this.openSelectedOriginal());
    });
    this.root.querySelectorAll<HTMLElement>('[data-action="open-original"]').forEach((element) => {
      element.addEventListener("click", () => void this.openSelectedOriginal());
    });
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

  private bindPullToRefresh(): void {
    const timeline = this.root.querySelector<HTMLElement>(".timeline");
    const indicator = timeline?.querySelector<HTMLElement>("[data-pull-refresh]");
    const view = this.root.ownerDocument.defaultView;
    if (
      !timeline ||
      !indicator ||
      this.refreshing ||
      !view?.matchMedia?.("(max-width: 720px)").matches
    ) return;

    const label = indicator.querySelector<HTMLElement>("[data-pull-refresh-label]");
    let startX: number | null = null;
    let startY: number | null = null;
    let distance = 0;

    const reset = () => {
      startX = null;
      startY = null;
      distance = 0;
      indicator.style.removeProperty("--pull-distance");
      indicator.classList.remove("visible", "ready");
      indicator.setAttribute("aria-hidden", "true");
      if (label) label.textContent = "Tirez pour actualiser";
    };

    timeline.addEventListener("touchstart", (event) => {
      const touch = event.touches[0];
      if (!touch || timeline.scrollTop > 0 || event.touches.length !== 1) return;
      startX = touch.clientX;
      startY = touch.clientY;
      distance = 0;
    }, { passive: true });

    timeline.addEventListener("touchmove", (event) => {
      const touch = event.touches[0];
      if (!touch || startX === null || startY === null) return;
      const horizontalDistance = Math.abs(touch.clientX - startX);
      const verticalDistance = touch.clientY - startY;
      if (horizontalDistance > Math.max(8, verticalDistance)) {
        reset();
        return;
      }
      if (verticalDistance <= 0 || timeline.scrollTop > 0) {
        reset();
        return;
      }
      event.preventDefault();
      distance = Math.min(PULL_REFRESH_MAX_DISTANCE, verticalDistance * 0.5);
      indicator.style.setProperty("--pull-distance", `${distance}px`);
      indicator.classList.toggle("visible", distance > 4);
      indicator.classList.toggle("ready", distance >= PULL_REFRESH_THRESHOLD);
      indicator.setAttribute("aria-hidden", distance > 4 ? "false" : "true");
      if (label) {
        label.textContent = distance >= PULL_REFRESH_THRESHOLD
          ? "Relâchez pour actualiser"
          : "Tirez pour actualiser";
      }
    }, { passive: false });

    timeline.addEventListener("touchend", () => {
      const shouldRefresh = distance >= PULL_REFRESH_THRESHOLD;
      reset();
      if (shouldRefresh) void this.refresh();
    });
    timeline.addEventListener("touchcancel", reset);
  }
}
