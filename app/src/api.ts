import { invoke } from "@tauri-apps/api/core";
import type {
  ArticleDetail,
  ArticleSummary,
  DeleteFeedResult,
  Feed,
  Platform,
  RefreshReport,
} from "./types";

export interface ReaderApi {
  listArticles(): Promise<ArticleSummary[]>;
  getArticle(articleId: string): Promise<ArticleDetail>;
  refreshFeeds(): Promise<RefreshReport>;
  setArticleRead(articleId: string, isRead: boolean): Promise<void>;
  setArticleFavorite(articleId: string, isFavorite: boolean): Promise<void>;
  listFeeds(): Promise<Feed[]>;
  addFeed(url: string, platform?: Platform): Promise<Feed>;
  setFeedActive(feedId: string, isActive: boolean): Promise<Feed>;
  deleteFeed(feedId: string): Promise<DeleteFeedResult>;
}

export const tauriApi: ReaderApi = {
  listArticles: () => invoke("list_articles"),
  getArticle: (articleId) => invoke("get_article", { articleId }),
  refreshFeeds: () => invoke("refresh_feeds"),
  setArticleRead: (articleId, isRead) =>
    invoke("set_article_read", { articleId, isRead }),
  setArticleFavorite: (articleId, isFavorite) =>
    invoke("set_article_favorite", { articleId, isFavorite }),
  listFeeds: () => invoke("list_feeds"),
  addFeed: (url, platform) => invoke("add_feed", { url, platform }),
  setFeedActive: (feedId, isActive) =>
    invoke("set_feed_active", { feedId, isActive }),
  deleteFeed: (feedId) => invoke("delete_feed", { feedId }),
};
