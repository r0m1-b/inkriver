export type Platform = "medium" | "substack" | "other";
export type ContentKind = "full" | "excerpt" | "missing" | "unknown";

export interface ArticleSummary {
  id: string;
  feedId: string;
  title: string | null;
  author: string | null;
  publishedAt: string | null;
  url: string | null;
  source: Platform;
  isRead: boolean;
  isFavorite: boolean;
}

export interface ArticleDetail extends ArticleSummary {
  content: string | null;
  contentKind: ContentKind;
}

export interface Feed {
  id: string;
  platform: Platform;
  url: string;
  isActive: boolean;
  title: string | null;
  description: string | null;
  author: string | null;
  lastPublishedAt: string | null;
  lastSuccessAt: string | null;
  lastError: FeedStatusError | null;
}

export interface FeedStatusError {
  stage: string;
  message: string;
  occurredAt: string;
}

export interface DeleteFeedResult {
  feedId: string;
  deletedArticles: number;
}

export interface FeedRefreshError {
  feedId: string;
  feedUrl: string;
  stage: string;
  message: string;
}

export interface RefreshReport {
  activeFeeds: number;
  collectedArticles: number;
  insertedArticles: number;
  updatedArticles: number;
  autoArchivedArticles: number;
  errors: FeedRefreshError[];
}

export interface ApiError {
  code: string;
  message: string;
}
