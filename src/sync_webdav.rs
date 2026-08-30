use crate::http::client_builder;
use crate::sync_acknowledgements::ACKNOWLEDGEMENT_DIRECTORY;
use crate::sync_roster::{MAX_ROSTER_BYTES, ROSTER_DIRECTORY};
use crate::sync_snapshots::{MAX_SNAPSHOT_BYTES, SNAPSHOT_DIRECTORY};
use crate::sync_transport::{SegmentPublishOutcome, SegmentTransport};
use anyhow::{Context, Result, bail};
use futures_util::stream::{self, StreamExt, TryStreamExt};
use quick_xml::Reader;
use quick_xml::events::Event;
use reqwest::header::{CONTENT_LENGTH, HeaderName, HeaderValue};
use reqwest::{Method, Response, StatusCode, Url};
use std::fmt;
use std::time::Duration;
use zeroize::Zeroizing;

const WEBDAV_TIMEOUT: Duration = Duration::from_secs(20);
const WEBDAV_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_PROPFIND_BYTES: usize = 1024 * 1024;
const MAX_WEBDAV_ENTRIES: usize = 1_000;
const MAX_CONCURRENT_REQUESTS: usize = 4;
const USER_AGENT: &str = "InkRiver/0.3 WebDAV";

const DEPTH: HeaderName = HeaderName::from_static("depth");
const DESTINATION: HeaderName = HeaderName::from_static("destination");
const OVERWRITE: HeaderName = HeaderName::from_static("overwrite");

/// Credentials and endpoint for one user-managed WebDAV directory.
#[derive(Clone)]
pub struct WebDavConfig {
    base_url: Url,
    username: String,
    password: Zeroizing<String>,
}

impl WebDavConfig {
    /// Creates a configuration rooted at one dedicated remote collection.
    pub fn new(base_url: &str, username: String, password: String) -> Result<Self> {
        let mut base_url = Url::parse(base_url).context("URL WebDAV invalide")?;
        if !matches!(base_url.scheme(), "http" | "https")
            || base_url.host_str().is_none()
            || !base_url.username().is_empty()
            || base_url.password().is_some()
            || base_url.query().is_some()
            || base_url.fragment().is_some()
        {
            bail!("WebDAV requires an HTTP(S) URL without embedded credentials, query or fragment");
        }
        if !base_url.path().ends_with('/') {
            let path = format!("{}/", base_url.path());
            base_url.set_path(&path);
        }
        Ok(Self {
            base_url,
            username,
            password: Zeroizing::new(password),
        })
    }
}

impl fmt::Debug for WebDavConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WebDavConfig")
            .field("base_url", &self.base_url)
            .field("username", &self.username)
            .field("password", &"[REDACTED]")
            .finish()
    }
}

/// Bounded WebDAV implementation of the encrypted segment transport.
pub struct WebDavTransport {
    config: WebDavConfig,
    client: reqwest::Client,
}

impl WebDavTransport {
    pub fn new(config: WebDavConfig) -> Result<Self> {
        let client = client_builder()
            .map_err(anyhow::Error::msg)?
            .connect_timeout(WEBDAV_CONNECT_TIMEOUT)
            .timeout(WEBDAV_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(USER_AGENT)
            .build()
            .context("Impossible de créer le client WebDAV")?;
        Ok(Self { config, client })
    }

    fn url(&self, relative_path: &str) -> Result<Url> {
        validate_relative_path(relative_path)?;
        self.config
            .base_url
            .join(relative_path)
            .context("Chemin WebDAV invalide")
    }

    fn request(&self, method: Method, url: Url) -> reqwest::RequestBuilder {
        self.client
            .request(method, url)
            .basic_auth(&self.config.username, Some(self.config.password.as_str()))
    }

    async fn ensure_collection(&self, relative_path: &str) -> Result<()> {
        let url = self.url(relative_path)?;
        let response = self
            .request(Method::from_bytes(b"MKCOL")?, url)
            .send()
            .await
            .context("Requête MKCOL impossible")?;
        if response.status().is_success() || response.status() == StatusCode::METHOD_NOT_ALLOWED {
            return Ok(());
        }
        bail!("MKCOL returned HTTP status {}", response.status())
    }

    async fn propfind(&self, relative_path: &str) -> Result<Vec<DavEntry>> {
        let response = self
            .request(Method::from_bytes(b"PROPFIND")?, self.url(relative_path)?)
            .header(DEPTH, HeaderValue::from_static("1"))
            .header("Content-Type", "application/xml; charset=utf-8")
            .body(
                r#"<?xml version="1.0" encoding="utf-8" ?>
                   <d:propfind xmlns:d="DAV:"><d:prop><d:resourcetype/></d:prop></d:propfind>"#,
            )
            .send()
            .await
            .context("Requête PROPFIND impossible")?;
        if response.status() != StatusCode::MULTI_STATUS {
            bail!("PROPFIND returned HTTP status {}", response.status());
        }
        let body = read_bounded(response, MAX_PROPFIND_BYTES).await?;
        parse_multistatus(&body)
    }

    async fn delete_temporary(&self, relative_path: &str) {
        if let Ok(url) = self.url(relative_path) {
            let _ = self.request(Method::DELETE, url).send().await;
        }
    }

    async fn publish_replaceable(&self, relative_path: &str, bytes: &[u8]) -> Result<()> {
        let (directory, file_name) = relative_path
            .rsplit_once('/')
            .context("Le chemin remplaçable n'a pas de répertoire")?;
        let temporary = format!("{directory}/.{file_name}.{}.tmp", uuid::Uuid::new_v4());
        let put = self
            .request(Method::PUT, self.url(&temporary)?)
            .header("Content-Type", "application/json")
            .body(bytes.to_vec())
            .send()
            .await
            .context("Téléversement temporaire WebDAV impossible")?;
        if !put.status().is_success() {
            bail!("Temporary PUT returned HTTP status {}", put.status());
        }
        let moved = self
            .request(Method::from_bytes(b"MOVE")?, self.url(&temporary)?)
            .header(DESTINATION, self.url(relative_path)?.as_str())
            .header(OVERWRITE, HeaderValue::from_static("T"))
            .send()
            .await;
        match moved {
            Ok(response) if response.status().is_success() => Ok(()),
            Ok(response) => {
                self.delete_temporary(&temporary).await;
                bail!("MOVE returned HTTP status {}", response.status())
            }
            Err(error) => {
                self.delete_temporary(&temporary).await;
                Err(error).context("Remplacement atomique WebDAV impossible")
            }
        }
    }

    fn relative_from_href(&self, href: &str) -> Result<String> {
        let url = self
            .config
            .base_url
            .join(href)
            .context("PROPFIND returned an invalid href")?;
        if url.scheme() != self.config.base_url.scheme()
            || url.host_str() != self.config.base_url.host_str()
            || url.port_or_known_default() != self.config.base_url.port_or_known_default()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            bail!("PROPFIND returned a href outside the configured WebDAV origin");
        }
        let relative = url
            .path()
            .strip_prefix(self.config.base_url.path())
            .context("PROPFIND returned a href outside the configured WebDAV directory")?;
        validate_relative_path(relative)?;
        Ok(relative.to_string())
    }

    async fn direct_children(&self, collection: &str) -> Result<Vec<(String, bool)>> {
        let prefix = collection.trim_end_matches('/');
        let prefix_with_slash = format!("{prefix}/");
        let mut children = Vec::new();
        for entry in self.propfind(collection).await? {
            let relative = self.relative_from_href(&entry.href)?;
            let normalized = relative.trim_end_matches('/');
            if normalized == prefix {
                continue;
            }
            let child = normalized
                .strip_prefix(&prefix_with_slash)
                .context("PROPFIND returned an entry outside the requested collection")?;
            if child.is_empty() || child.contains('/') {
                bail!("PROPFIND returned a non-direct child");
            }
            if !child.starts_with('.') {
                children.push((child.to_string(), entry.is_collection));
            }
            if children.len() > MAX_WEBDAV_ENTRIES {
                bail!("WebDAV collection contains too many entries");
            }
        }
        children.sort();
        children.dedup();
        Ok(children)
    }
}

impl SegmentTransport for WebDavTransport {
    async fn ensure_layout(&self, key_id: &str, device_id: &str) -> Result<()> {
        validate_key_and_device(key_id, device_id)?;
        self.ensure_collection("").await?;
        self.ensure_collection("v2/").await?;
        self.ensure_collection(&format!("v2/{key_id}/")).await?;
        self.ensure_collection(&format!("v2/{key_id}/{ACKNOWLEDGEMENT_DIRECTORY}/"))
            .await?;
        self.ensure_collection(&format!("v2/{key_id}/{SNAPSHOT_DIRECTORY}/"))
            .await?;
        self.ensure_collection(&format!("v2/{key_id}/{ROSTER_DIRECTORY}/"))
            .await?;
        self.ensure_collection(&format!("v2/{key_id}/{device_id}/"))
            .await
    }

    async fn publish_immutable(
        &self,
        relative_path: &str,
        bytes: &[u8],
    ) -> Result<SegmentPublishOutcome> {
        if bytes.len() > crate::sync_segments::MAX_SEGMENT_BYTES as usize {
            bail!("Synchronization segment exceeds the upload limit");
        }
        let (directory, file_name) = relative_path
            .rsplit_once('/')
            .context("Segment path has no device directory")?;
        let temporary = format!("{directory}/.{file_name}.{}.tmp", uuid::Uuid::new_v4());
        let put = self
            .request(Method::PUT, self.url(&temporary)?)
            .header("Content-Type", "application/json")
            .body(bytes.to_vec())
            .send()
            .await
            .context("Téléversement temporaire WebDAV impossible")?;
        if !put.status().is_success() {
            bail!("Temporary PUT returned HTTP status {}", put.status());
        }

        let destination = self.url(relative_path)?;
        let moved = self
            .request(Method::from_bytes(b"MOVE")?, self.url(&temporary)?)
            .header(DESTINATION, destination.as_str())
            .header(OVERWRITE, HeaderValue::from_static("F"))
            .send()
            .await;
        match moved {
            Ok(response) if response.status().is_success() => Ok(SegmentPublishOutcome::Created),
            Ok(response)
                if matches!(
                    response.status(),
                    StatusCode::PRECONDITION_FAILED | StatusCode::CONFLICT
                ) =>
            {
                self.delete_temporary(&temporary).await;
                Ok(SegmentPublishOutcome::AlreadyExists)
            }
            Ok(response) => {
                self.delete_temporary(&temporary).await;
                bail!("MOVE returned HTTP status {}", response.status())
            }
            Err(error) => {
                self.delete_temporary(&temporary).await;
                Err(error).context("Publication atomique WebDAV impossible")
            }
        }
    }

    async fn list_segments(&self, key_id: &str) -> Result<Vec<String>> {
        if key_id.len() != 64 || !key_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            bail!("Invalid synchronization key identifier");
        }
        let key_collection = format!("v2/{key_id}/");
        let devices = self.direct_children(&key_collection).await?;
        let devices = devices
            .into_iter()
            .filter(|(name, _)| {
                name != ACKNOWLEDGEMENT_DIRECTORY
                    && name != SNAPSHOT_DIRECTORY
                    && name != ROSTER_DIRECTORY
            })
            .collect::<Vec<_>>();
        for (device, is_collection) in &devices {
            if !is_collection || uuid::Uuid::parse_str(device).is_err() {
                bail!("WebDAV key directory contains an invalid device entry");
            }
        }
        let lists = stream::iter(devices.into_iter().map(|(device, _)| {
            let collection = format!("{key_collection}{device}/");
            async move {
                let children = self.direct_children(&collection).await?;
                let mut paths = Vec::new();
                for (name, is_collection) in children {
                    if is_collection || !is_segment_file_name(&name) {
                        bail!("WebDAV device directory contains an invalid segment entry");
                    }
                    paths.push(format!("{collection}{name}"));
                }
                Ok::<_, anyhow::Error>(paths)
            }
        }))
        .buffer_unordered(MAX_CONCURRENT_REQUESTS)
        .try_collect::<Vec<_>>()
        .await?;
        let mut paths = lists.into_iter().flatten().collect::<Vec<_>>();
        if paths.len() > MAX_WEBDAV_ENTRIES {
            bail!("WebDAV synchronization contains too many segments");
        }
        paths.sort();
        Ok(paths)
    }

    async fn download_segment(&self, relative_path: &str, max_bytes: usize) -> Result<Vec<u8>> {
        let response = self
            .request(Method::GET, self.url(relative_path)?)
            .send()
            .await
            .context("Téléchargement WebDAV impossible")?;
        if !response.status().is_success() {
            bail!("GET returned HTTP status {}", response.status());
        }
        read_bounded(response, max_bytes).await
    }

    async fn delete_segment(&self, relative_path: &str) -> Result<()> {
        validate_segment_path(relative_path)?;
        let response = self
            .request(Method::DELETE, self.url(relative_path)?)
            .send()
            .await
            .context("Suppression WebDAV impossible")?;
        if response.status().is_success() || response.status() == StatusCode::NOT_FOUND {
            return Ok(());
        }
        bail!("DELETE returned HTTP status {}", response.status())
    }

    async fn publish_acknowledgement(&self, relative_path: &str, bytes: &[u8]) -> Result<()> {
        if bytes.len() > crate::sync_acknowledgements::MAX_ACKNOWLEDGEMENT_BYTES {
            bail!("Synchronization acknowledgement exceeds the upload limit");
        }
        self.publish_replaceable(relative_path, bytes).await
    }

    async fn list_acknowledgements(&self, key_id: &str) -> Result<Vec<String>> {
        validate_key_id(key_id)?;
        let collection = format!("v2/{key_id}/{ACKNOWLEDGEMENT_DIRECTORY}/");
        let mut paths = Vec::new();
        for (name, is_collection) in self.direct_children(&collection).await? {
            let observer = name
                .strip_suffix(".json")
                .context("Invalid synchronization acknowledgement file")?;
            if is_collection || uuid::Uuid::parse_str(observer).is_err() {
                bail!("Invalid synchronization acknowledgement file");
            }
            paths.push(format!("{collection}{name}"));
        }
        Ok(paths)
    }

    async fn download_acknowledgement(
        &self,
        relative_path: &str,
        max_bytes: usize,
    ) -> Result<Vec<u8>> {
        self.download_segment(relative_path, max_bytes).await
    }

    async fn publish_snapshot(&self, relative_path: &str, bytes: &[u8]) -> Result<()> {
        if bytes.len() > MAX_SNAPSHOT_BYTES {
            bail!("Synchronization snapshot exceeds the upload limit");
        }
        self.publish_replaceable(relative_path, bytes).await
    }

    async fn list_snapshots(&self, key_id: &str) -> Result<Vec<String>> {
        validate_key_id(key_id)?;
        let collection = format!("v2/{key_id}/{SNAPSHOT_DIRECTORY}/");
        let mut paths = Vec::new();
        for (name, is_collection) in self.direct_children(&collection).await? {
            let creator = name
                .strip_suffix(".json")
                .context("Invalid synchronization snapshot file")?;
            if is_collection || uuid::Uuid::parse_str(creator).is_err() {
                bail!("Invalid synchronization snapshot file");
            }
            paths.push(format!("{collection}{name}"));
        }
        Ok(paths)
    }

    async fn download_snapshot(&self, relative_path: &str, max_bytes: usize) -> Result<Vec<u8>> {
        self.download_segment(relative_path, max_bytes).await
    }

    async fn publish_roster(&self, relative_path: &str, bytes: &[u8]) -> Result<()> {
        if bytes.len() > MAX_ROSTER_BYTES {
            bail!("Synchronization roster exceeds the upload limit");
        }
        self.publish_replaceable(relative_path, bytes).await
    }

    async fn list_rosters(&self, key_id: &str) -> Result<Vec<String>> {
        validate_key_id(key_id)?;
        let collection = format!("v2/{key_id}/{ROSTER_DIRECTORY}/");
        let mut paths = Vec::new();
        for (name, is_collection) in self.direct_children(&collection).await? {
            let publisher = name
                .strip_suffix(".json")
                .context("Invalid synchronization roster file")?;
            if is_collection || uuid::Uuid::parse_str(publisher).is_err() {
                bail!("Invalid synchronization roster file");
            }
            paths.push(format!("{collection}{name}"));
        }
        Ok(paths)
    }

    async fn download_roster(&self, relative_path: &str, max_bytes: usize) -> Result<Vec<u8>> {
        self.download_segment(relative_path, max_bytes).await
    }
}

#[derive(Debug)]
struct DavEntry {
    href: String,
    is_collection: bool,
}

fn parse_multistatus(bytes: &[u8]) -> Result<Vec<DavEntry>> {
    let mut reader = Reader::from_reader(bytes);
    reader.config_mut().trim_text(true);
    let mut entries = Vec::new();
    let mut current_href = None;
    let mut current_collection = false;
    loop {
        match reader.read_event().context("Réponse XML WebDAV invalide")? {
            Event::Start(event) => {
                let local_name = event.local_name();
                if local_name.as_ref() == b"response" {
                    current_href = None;
                    current_collection = false;
                } else if local_name.as_ref() == b"href" {
                    let text = reader
                        .read_text(event.name())
                        .context("href WebDAV invalide")?;
                    let decoded = text.decode().context("href WebDAV non décodable")?;
                    current_href = Some(
                        quick_xml::escape::unescape(&decoded)
                            .context("href WebDAV invalide")?
                            .into_owned(),
                    );
                } else if local_name.as_ref() == b"collection" {
                    current_collection = true;
                }
            }
            Event::Empty(event) if event.local_name().as_ref() == b"collection" => {
                current_collection = true;
            }
            Event::End(event) => {
                if event.local_name().as_ref() == b"response" {
                    entries.push(DavEntry {
                        href: current_href.take().context("Réponse WebDAV sans href")?,
                        is_collection: current_collection,
                    });
                }
            }
            Event::Eof => break,
            _ => {}
        }
        if entries.len() > MAX_WEBDAV_ENTRIES + 1 {
            bail!("Réponse WebDAV trop volumineuse");
        }
    }
    Ok(entries)
}

async fn read_bounded(mut response: Response, max_bytes: usize) -> Result<Vec<u8>> {
    if let Some(length) = response
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        && length > max_bytes as u64
    {
        bail!("WebDAV response exceeds the size limit");
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .context("Réponse WebDAV interrompue")?
    {
        if bytes.len().saturating_add(chunk.len()) > max_bytes {
            bail!("WebDAV response exceeds the size limit");
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn validate_relative_path(path: &str) -> Result<()> {
    if path.starts_with('/')
        || path.contains('\\')
        || path.contains('?')
        || path.contains('#')
        || path.split('/').any(|part| matches!(part, "." | ".."))
    {
        bail!("Invalid relative WebDAV path");
    }
    Ok(())
}

fn validate_key_and_device(key_id: &str, device_id: &str) -> Result<()> {
    validate_key_id(key_id)?;
    uuid::Uuid::parse_str(device_id).context("Invalid synchronization device identifier")?;
    Ok(())
}

fn validate_key_id(key_id: &str) -> Result<()> {
    if key_id.len() != 64
        || !key_id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        bail!("Invalid synchronization key identifier");
    }
    Ok(())
}

fn is_segment_file_name(name: &str) -> bool {
    let Some(range) = name.strip_suffix(".json") else {
        return false;
    };
    let Some((first, last)) = range.split_once('-') else {
        return false;
    };
    if first.len() != 20
        || last.len() != 20
        || !first.bytes().all(|byte| byte.is_ascii_digit())
        || !last.bytes().all(|byte| byte.is_ascii_digit())
    {
        return false;
    }
    let (Ok(first), Ok(last)) = (first.parse::<i64>(), last.parse::<i64>()) else {
        return false;
    };
    first > 0 && last >= first
}

fn validate_segment_path(relative_path: &str) -> Result<()> {
    validate_relative_path(relative_path)?;
    let components = relative_path.split('/').collect::<Vec<_>>();
    if components.len() != 4 || components[0] != "v2" {
        bail!("Invalid synchronization segment path");
    }
    validate_key_and_device(components[1], components[2])?;
    if !is_segment_file_name(components[3]) {
        bail!("Invalid synchronization segment path");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::article::{Article, ContentKind, Source};
    use crate::storage::Storage;
    use crate::sync_segments::SyncGroupKey;
    use crate::sync_transport::synchronize_transport;
    use chrono::Utc;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    #[derive(Clone)]
    enum Node {
        Collection,
        File(Vec<u8>),
    }

    struct FakeState {
        nodes: Mutex<HashMap<String, Node>>,
        drop_next_move_response: AtomicBool,
        saw_authorization: AtomicBool,
    }

    struct FakeWebDav {
        base_url: String,
        state: Arc<FakeState>,
        task: tokio::task::JoinHandle<()>,
    }

    impl FakeWebDav {
        async fn start() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let state = Arc::new(FakeState {
                nodes: Mutex::new(HashMap::from([("/dav/".to_string(), Node::Collection)])),
                drop_next_move_response: AtomicBool::new(false),
                saw_authorization: AtomicBool::new(false),
            });
            let server_state = Arc::clone(&state);
            let task = tokio::spawn(async move {
                loop {
                    let Ok((stream, _)) = listener.accept().await else {
                        break;
                    };
                    let request_state = Arc::clone(&server_state);
                    tokio::spawn(async move {
                        let _ = handle_connection(stream, request_state).await;
                    });
                }
            });
            Self {
                base_url: format!("http://{address}/dav/inkriver/"),
                state,
                task,
            }
        }

        fn transport(&self) -> WebDavTransport {
            WebDavTransport::new(
                WebDavConfig::new(&self.base_url, "user".into(), "pass".into()).unwrap(),
            )
            .unwrap()
        }

        fn drop_next_move_response(&self) {
            self.state
                .drop_next_move_response
                .store(true, Ordering::SeqCst);
        }

        fn stop(&self) {
            self.task.abort();
        }

        fn encrypted_files(&self) -> Vec<Vec<u8>> {
            self.state
                .nodes
                .lock()
                .unwrap()
                .values()
                .filter_map(|node| match node {
                    Node::File(bytes) => Some(bytes.clone()),
                    Node::Collection => None,
                })
                .collect()
        }
    }

    impl Drop for FakeWebDav {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    struct Request {
        method: String,
        path: String,
        headers: HashMap<String, String>,
        body: Vec<u8>,
    }

    async fn handle_connection(mut stream: TcpStream, state: Arc<FakeState>) -> Result<()> {
        let request = read_request(&mut stream).await?;
        if request.headers.get("authorization") == Some(&"Basic dXNlcjpwYXNz".to_string()) {
            state.saw_authorization.store(true, Ordering::SeqCst);
        } else {
            write_response(&mut stream, 401, "Unauthorized", Vec::new(), "text/plain").await?;
            return Ok(());
        }

        let (status, reason, body, content_type, drop_response) = {
            let mut nodes = state.nodes.lock().unwrap();
            match request.method.as_str() {
                "MKCOL" => {
                    let path = collection_path(&request.path);
                    if nodes.contains_key(&path) {
                        (405, "Method Not Allowed", Vec::new(), "text/plain", false)
                    } else if nodes.contains_key(parent_collection(&path)) {
                        nodes.insert(path, Node::Collection);
                        (201, "Created", Vec::new(), "text/plain", false)
                    } else {
                        (409, "Conflict", Vec::new(), "text/plain", false)
                    }
                }
                "PUT" => {
                    if nodes.contains_key(parent_collection(&request.path)) {
                        nodes.insert(request.path.clone(), Node::File(request.body));
                        (201, "Created", Vec::new(), "text/plain", false)
                    } else {
                        (409, "Conflict", Vec::new(), "text/plain", false)
                    }
                }
                "MOVE" => {
                    let destination = request
                        .headers
                        .get("destination")
                        .and_then(|value| Url::parse(value).ok())
                        .map(|url| url.path().to_string());
                    if let Some(destination) = destination {
                        let overwrite = request.headers.get("overwrite").map(String::as_str);
                        if nodes.contains_key(&destination) && overwrite != Some("T") {
                            (412, "Precondition Failed", Vec::new(), "text/plain", false)
                        } else if let Some(node) = nodes.remove(&request.path) {
                            nodes.insert(destination, node);
                            let drop_response =
                                state.drop_next_move_response.swap(false, Ordering::SeqCst);
                            (201, "Created", Vec::new(), "text/plain", drop_response)
                        } else {
                            (404, "Not Found", Vec::new(), "text/plain", false)
                        }
                    } else {
                        (400, "Bad Request", Vec::new(), "text/plain", false)
                    }
                }
                "DELETE" => {
                    if nodes.remove(&request.path).is_some() {
                        (204, "No Content", Vec::new(), "text/plain", false)
                    } else {
                        (404, "Not Found", Vec::new(), "text/plain", false)
                    }
                }
                "GET" => match nodes.get(&request.path) {
                    Some(Node::File(bytes)) => {
                        (200, "OK", bytes.clone(), "application/json", false)
                    }
                    _ => (404, "Not Found", Vec::new(), "text/plain", false),
                },
                "PROPFIND" => {
                    let path = collection_path(&request.path);
                    if nodes.contains_key(&path) {
                        let xml = multistatus(&path, &nodes).into_bytes();
                        (207, "Multi-Status", xml, "application/xml", false)
                    } else {
                        (404, "Not Found", Vec::new(), "text/plain", false)
                    }
                }
                _ => (405, "Method Not Allowed", Vec::new(), "text/plain", false),
            }
        };
        if !drop_response {
            write_response(&mut stream, status, reason, body, content_type).await?;
        }
        Ok(())
    }

    async fn read_request(stream: &mut TcpStream) -> Result<Request> {
        let mut bytes = Vec::new();
        let header_end = loop {
            if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                break index + 4;
            }
            if bytes.len() > 3 * 1024 * 1024 {
                bail!("test request too large");
            }
            let mut chunk = [0_u8; 4096];
            let read = stream.read(&mut chunk).await?;
            if read == 0 {
                bail!("test request ended before its headers");
            }
            bytes.extend_from_slice(&chunk[..read]);
        };
        let headers_text = std::str::from_utf8(&bytes[..header_end])?;
        let mut lines = headers_text.split("\r\n");
        let request_line = lines.next().context("missing request line")?;
        let mut request_parts = request_line.split_whitespace();
        let method = request_parts.next().context("missing method")?.to_string();
        let path = request_parts.next().context("missing path")?.to_string();
        let mut headers = HashMap::new();
        for line in lines.filter(|line| !line.is_empty()) {
            let (name, value) = line.split_once(':').context("invalid test header")?;
            headers.insert(name.to_ascii_lowercase(), value.trim().to_string());
        }
        let content_length = headers
            .get("content-length")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        while bytes.len() < header_end + content_length {
            let mut chunk = [0_u8; 4096];
            let read = stream.read(&mut chunk).await?;
            if read == 0 {
                bail!("test request body was truncated");
            }
            bytes.extend_from_slice(&chunk[..read]);
        }
        Ok(Request {
            method,
            path,
            headers,
            body: bytes[header_end..header_end + content_length].to_vec(),
        })
    }

    async fn write_response(
        stream: &mut TcpStream,
        status: u16,
        reason: &str,
        body: Vec<u8>,
        content_type: &str,
    ) -> Result<()> {
        let headers = format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nContent-Type: {content_type}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(headers.as_bytes()).await?;
        stream.write_all(&body).await?;
        stream.shutdown().await?;
        Ok(())
    }

    fn collection_path(path: &str) -> String {
        if path.ends_with('/') {
            path.to_string()
        } else {
            format!("{path}/")
        }
    }

    fn parent_collection(path: &str) -> &str {
        let trimmed = path.trim_end_matches('/');
        let index = trimmed.rfind('/').unwrap();
        &path[..=index]
    }

    fn multistatus(collection: &str, nodes: &HashMap<String, Node>) -> String {
        let mut paths = vec![collection.to_string()];
        for path in nodes.keys() {
            if path == collection || !path.starts_with(collection) {
                continue;
            }
            let remainder = path[collection.len()..].trim_end_matches('/');
            if !remainder.is_empty() && !remainder.contains('/') {
                paths.push(path.clone());
            }
        }
        paths.sort();
        paths.dedup();
        let responses = paths
            .into_iter()
            .map(|path| {
                let collection_marker = if matches!(nodes.get(&path), Some(Node::Collection)) {
                    "<d:collection/>"
                } else {
                    ""
                };
                format!(
                    "<d:response><d:href>{path}</d:href><d:propstat><d:prop><d:resourcetype>{collection_marker}</d:resourcetype></d:prop></d:propstat></d:response>"
                )
            })
            .collect::<String>();
        format!(
            "<?xml version=\"1.0\"?><d:multistatus xmlns:d=\"DAV:\">{responses}</d:multistatus>"
        )
    }

    fn article(feed_id: &str) -> Article {
        Article {
            id: format!("{feed_id}::webdav-entry"),
            feed_id: feed_id.to_string(),
            title: Some("A private WebDAV test article".to_string()),
            author: Some("InkRiver".to_string()),
            published_at: Some(Utc::now()),
            url: Some("https://private.example/article".to_string()),
            content: Some("cached body".to_string()),
            content_kind: ContentKind::Full,
            source: Source::Other,
        }
    }

    async fn single_response_server(response: &'static [u8]) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 4096];
            let _ = stream.read(&mut request).await;
            stream.write_all(response).await.unwrap();
            stream.shutdown().await.unwrap();
        });
        format!("http://{address}/dav/")
    }

    #[test]
    fn configuration_rejects_unsafe_urls_and_redacts_passwords() {
        assert!(WebDavConfig::new("ftp://example.test/dav", "u".into(), "p".into()).is_err());
        assert!(WebDavConfig::new("https://u:p@example.test/dav", "u".into(), "p".into()).is_err());
        let config = WebDavConfig::new(
            "https://example.test/dav",
            "reader".into(),
            "very-secret".into(),
        )
        .unwrap();
        let debug = format!("{config:?}");
        assert!(!debug.contains("very-secret"));
        assert!(debug.contains("[REDACTED]"));
    }

    #[test]
    fn multistatus_parser_handles_namespaces_entities_and_collections() {
        let xml = br#"<?xml version="1.0"?>
            <d:multistatus xmlns:d="DAV:">
              <d:response><d:href>/dav/a&amp;b/</d:href><d:propstat><d:prop>
                <d:resourcetype><d:collection/></d:resourcetype>
              </d:prop></d:propstat></d:response>
            </d:multistatus>"#;
        let entries = parse_multistatus(xml).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].href, "/dav/a&b/");
        assert!(entries[0].is_collection);
    }

    #[tokio::test]
    async fn segment_deletion_is_idempotent_and_rejects_every_other_path() {
        let server = FakeWebDav::start().await;
        let transport = server.transport();
        let key_id = "12".repeat(32);
        let device_id = "00000000-0000-4000-8000-000000000012";
        let path =
            format!("v2/{key_id}/{device_id}/00000000000000000001-00000000000000000002.json");
        transport.ensure_layout(&key_id, device_id).await.unwrap();
        assert_eq!(
            transport.publish_immutable(&path, b"{}").await.unwrap(),
            SegmentPublishOutcome::Created
        );

        transport.delete_segment(&path).await.unwrap();
        transport.delete_segment(&path).await.unwrap();
        assert!(
            !transport
                .list_segments(&key_id)
                .await
                .unwrap()
                .contains(&path)
        );
        for invalid in [
            format!("v2/{key_id}/snapshots/{device_id}.json"),
            format!("v2/{key_id}/{device_id}/../secret.json"),
            format!("v2/{key_id}/{device_id}/not-a-segment.json"),
        ] {
            assert!(transport.delete_segment(&invalid).await.is_err());
        }
    }

    #[tokio::test]
    async fn downloads_reject_redirects_and_declared_oversized_bodies() {
        let redirect_url = single_response_server(
            b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:9/leak\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        )
        .await;
        let redirect = WebDavTransport::new(
            WebDavConfig::new(&redirect_url, "user".into(), "pass".into()).unwrap(),
        )
        .unwrap()
        .download_segment("v2/segment.json", 10)
        .await
        .unwrap_err()
        .to_string();
        assert!(redirect.contains("302"));

        let oversized_url = single_response_server(
            b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\n",
        )
        .await;
        let oversized = WebDavTransport::new(
            WebDavConfig::new(&oversized_url, "user".into(), "pass".into()).unwrap(),
        )
        .unwrap()
        .download_segment("v2/segment.json", 10)
        .await
        .unwrap_err()
        .to_string();
        assert!(oversized.contains("size limit"));
    }

    #[tokio::test]
    async fn unavailable_webdav_does_not_block_local_storage() {
        let server = FakeWebDav::start().await;
        let transport = server.transport();
        let storage = Storage::open_in_memory().await.unwrap();
        storage.enable_sync().await.unwrap();
        let feed = storage
            .add_feed("https://offline.example/feed", None)
            .await
            .unwrap();
        server.stop();
        tokio::task::yield_now().await;
        assert!(
            synchronize_transport(
                &storage,
                &SyncGroupKey::from_bytes([0x71; 32]),
                &transport,
                Utc::now(),
            )
            .await
            .is_err()
        );
        storage.set_feed_active(&feed.id, false).await.unwrap();
        assert!(!storage.list_feeds().await.unwrap()[0].is_active);
    }

    #[tokio::test]
    async fn two_clients_converge_after_an_interrupted_atomic_publish() {
        let server = FakeWebDav::start().await;
        let key = SyncGroupKey::from_bytes([0x37; 32]);
        let linux = Storage::open_in_memory().await.unwrap();
        linux.enable_sync().await.unwrap();
        let linux_id = linux.sync_identity().await.unwrap().device_id;
        let feed = linux
            .add_feed("https://private.example/feed", None)
            .await
            .unwrap();
        let article = article(&feed.id);
        linux
            .upsert_articles(std::slice::from_ref(&article))
            .await
            .unwrap();
        linux.set_read(&article.id, true).await.unwrap();

        let first = synchronize_transport(&linux, &key, &server.transport(), Utc::now())
            .await
            .unwrap();
        assert_eq!(first.uploaded_segments, 1);
        assert_eq!(first.downloaded_segments, 0);
        let android = Storage::open_in_memory().await.unwrap();
        android.enable_sync().await.unwrap();
        let android_id = android.sync_identity().await.unwrap().device_id;
        let received = synchronize_transport(&android, &key, &server.transport(), Utc::now())
            .await
            .unwrap();
        assert_eq!(received.imported_events, 2);
        assert_eq!(received.downloaded_segments, 0);
        let android_article = android.list_articles().await.unwrap().pop().unwrap();
        android
            .set_read(&android_article.article.id, false)
            .await
            .unwrap();
        android
            .set_favorite(&android_article.article.id, true)
            .await
            .unwrap();

        server.drop_next_move_response();
        assert!(
            synchronize_transport(&android, &key, &server.transport(), Utc::now())
                .await
                .is_err()
        );
        let recovered = synchronize_transport(&android, &key, &server.transport(), Utc::now())
            .await
            .unwrap();
        assert_eq!(recovered.reused_segments, 1);
        assert_eq!(recovered.exported_events, 2);

        let merged = synchronize_transport(&linux, &key, &server.transport(), Utc::now())
            .await
            .unwrap();
        assert_eq!(merged.imported_events, 2);
        assert_eq!(merged.downloaded_segments, 0);
        let linux_article = linux.list_articles().await.unwrap().pop().unwrap();
        assert!(!linux_article.is_read);
        assert!(linux_article.is_favorite);
        let repeated = synchronize_transport(&linux, &key, &server.transport(), Utc::now())
            .await
            .unwrap();
        assert_eq!(repeated.downloaded_segments, 0);
        assert_eq!(repeated.imported_events, 0);
        assert!(
            linux
                .sync_acknowledgements_for_source(&key.key_id(), &linux_id)
                .await
                .unwrap()
                .iter()
                .any(|acknowledgement| {
                    acknowledgement.observer_device_id == android_id
                        && acknowledgement.contiguous_sequence >= 2
                })
        );
        let acknowledgement_files = server
            .state
            .nodes
            .lock()
            .unwrap()
            .keys()
            .filter(|path| path.contains("/acknowledgements/") && path.ends_with(".json"))
            .count();
        assert_eq!(acknowledgement_files, 2);
        let snapshot_files = server
            .state
            .nodes
            .lock()
            .unwrap()
            .keys()
            .filter(|path| path.contains("/snapshots/") && path.ends_with(".json"))
            .count();
        assert_eq!(snapshot_files, 2);
        let roster_files = server
            .state
            .nodes
            .lock()
            .unwrap()
            .keys()
            .filter(|path| path.contains("/rosters/") && path.ends_with(".json"))
            .count();
        assert_eq!(roster_files, 2);
        assert!(server.state.saw_authorization.load(Ordering::SeqCst));
        for bytes in server.encrypted_files() {
            let visible = String::from_utf8_lossy(&bytes);
            assert!(!visible.contains("private.example"));
            assert!(!visible.contains("A private WebDAV test article"));
            assert!(!visible.contains("article_favorite_set"));
        }
    }
}
