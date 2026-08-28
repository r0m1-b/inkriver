import { invoke } from "@tauri-apps/api/core";
import type {
  ArticleDetail,
  ArticleSummary,
  DeleteFeedResult,
  Feed,
  Platform,
  RefreshReport,
  PairingInvitation,
  SyncPairingStatus,
  SyncTransportReport,
} from "./types";

export interface InkRiverApi {
  listArticles(): Promise<ArticleSummary[]>;
  getArticle(articleId: string): Promise<ArticleDetail>;
  refreshFeeds(): Promise<RefreshReport>;
  refreshFeed(feedId: string): Promise<RefreshReport>;
  setArticleRead(articleId: string, isRead: boolean): Promise<void>;
  setArticlesRead(articleIds: string[], isRead: boolean): Promise<void>;
  setArticleFavorite(articleId: string, isFavorite: boolean): Promise<void>;
  archiveArticle(articleId: string): Promise<void>;
  archiveArticles(articleIds: string[]): Promise<void>;
  listFeeds(): Promise<Feed[]>;
  addFeed(url: string, platform?: Platform): Promise<Feed>;
  setFeedActive(feedId: string, isActive: boolean): Promise<Feed>;
  deleteFeed(feedId: string): Promise<DeleteFeedResult>;
  syncPairingStatus(): Promise<SyncPairingStatus>;
  configureSyncGroup(
    webdavBaseUrl: string,
    webdavUsername: string,
    webdavPassword: string,
    deviceName: string,
  ): Promise<SyncPairingStatus>;
  pairingInvitation(): Promise<PairingInvitation>;
  joinSyncGroup(
    invitation: string,
    webdavPassword: string,
    deviceName: string,
  ): Promise<SyncPairingStatus>;
  renameSyncDevice(deviceId: string, displayName: string): Promise<SyncPairingStatus>;
  revokeSyncDevice(deviceId: string): Promise<SyncPairingStatus>;
  synchronizeNow(): Promise<SyncTransportReport>;
  deleteSyncConfiguration(): Promise<SyncPairingStatus>;
}

export const tauriApi: InkRiverApi = {
  listArticles: () => invoke("list_articles"),
  getArticle: (articleId) => invoke("get_article", { articleId }),
  refreshFeeds: () => invoke("refresh_feeds"),
  refreshFeed: (feedId) => invoke("refresh_feed", { feedId }),
  setArticleRead: (articleId, isRead) =>
    invoke("set_article_read", { articleId, isRead }),
  setArticlesRead: (articleIds, isRead) =>
    invoke("set_articles_read", { articleIds, isRead }),
  setArticleFavorite: (articleId, isFavorite) =>
    invoke("set_article_favorite", { articleId, isFavorite }),
  archiveArticle: (articleId) => invoke("archive_article", { articleId }),
  archiveArticles: (articleIds) => invoke("archive_articles", { articleIds }),
  listFeeds: () => invoke("list_feeds"),
  addFeed: (url, platform) => invoke("add_feed", { url, platform }),
  setFeedActive: (feedId, isActive) =>
    invoke("set_feed_active", { feedId, isActive }),
  deleteFeed: (feedId) => invoke("delete_feed", { feedId }),
  syncPairingStatus: () => invoke("sync_pairing_status"),
  configureSyncGroup: (webdavBaseUrl, webdavUsername, webdavPassword, deviceName) =>
    invoke("configure_sync_group", {
      webdavBaseUrl,
      webdavUsername,
      webdavPassword,
      deviceName,
    }),
  pairingInvitation: () => invoke("pairing_invitation"),
  joinSyncGroup: (invitation, webdavPassword, deviceName) =>
    invoke("join_sync_group", { invitation, webdavPassword, deviceName }),
  renameSyncDevice: (deviceId, displayName) =>
    invoke("rename_sync_device", { deviceId, displayName }),
  revokeSyncDevice: (deviceId) => invoke("revoke_sync_device", { deviceId }),
  synchronizeNow: () => invoke("synchronize_now"),
  deleteSyncConfiguration: () => invoke("delete_sync_configuration"),
};
