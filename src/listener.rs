use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

pub type PlatformListener = crate::platform::MediaListener;
pub type WebsocketListener = crate::ws::MediaListener;
pub type WebsocketListenerPooled = crate::ws::MediaListenerPooled;

use crate::{Error, MediaMetadata, Result, TokioHandle, TokioRuntime};

#[derive(Debug)]
#[allow(unused)]
pub struct MediaListener {
  handle: TokioHandle,
  _runtime: Option<Arc<TokioRuntime>>,
  priority: Arc<RwLock<MediaListenerPriority>>,
  hybrid: Arc<AtomicBool>,
  platform_listener: Option<PlatformListener>,
  websocket_listener: Option<WebsocketListenerPooled>,
}

#[derive(
  Default, Debug, Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Hash, Serialize, Deserialize,
)]
pub enum WebsocketMode {
  Local(u16),
  Addr(SocketAddr),

  #[default]
  Default,
}

#[derive(
  Default, Debug, Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Hash, Serialize, Deserialize,
)]
pub enum MediaListenerPriority {
  #[default]
  Websocket,
  System,
}

#[derive(Default, Debug)]
pub struct MediaListenerConfig {
  pub handle: Option<TokioHandle>,
  pub ws: WebsocketMode,
  pub priority: MediaListenerPriority,
  pub hybrid: bool,
}

impl MediaListener {
  //noinspection DuplicatedCode
  pub fn new(cfg: MediaListenerConfig) -> Result<Self> {
    let handle = cfg
      .handle
      .clone()
      .or_else(|| TokioHandle::try_current().ok());

    let (handle, runtime) = match handle {
      Some(handle) => (handle, None),
      None => {
        let runtime = TokioRuntime::new().unwrap();

        (runtime.handle().clone(), Some(Arc::new(runtime)))
      }
    };

    let this_new = Self::new_async(cfg);
    let this = handle.block_on(this_new);

    match this {
      Ok(mut this) => {
        this._runtime = runtime;
        Ok(this)
      }
      Err(err) => Err(err),
    }
  }

  //noinspection DuplicatedCode
  pub async fn new_async(cfg: MediaListenerConfig) -> Result<Self> {
    let websocket_listener = match cfg.ws {
      WebsocketMode::Local(port) => WebsocketListener::bind_local(port).await,
      WebsocketMode::Addr(addr) => WebsocketListener::bind(addr).await,
      WebsocketMode::Default => WebsocketListener::bind_default().await,
    };

    let platform_listener = PlatformListener::new_async().await;

    // can't use pattern matching cause it moves the value,
    // and std::io::Error doesn't impl Clone
    if websocket_listener.is_err() && platform_listener.is_err() {
      let err1 = Box::new(Error::from(websocket_listener.unwrap_err()));
      let err2 = Box::new(platform_listener.unwrap_err());

      return Err(Error::FailedToCreateListener((err1, err2)));
    }

    let websocket_listener = websocket_listener.and_then(WebsocketListenerPooled::new);

    let handle = cfg.handle.or_else(|| TokioHandle::try_current().ok());

    let (handle, runtime) = match handle {
      Some(handle) => (handle, None),
      None => {
        let runtime = TokioRuntime::new().unwrap();

        (runtime.handle().clone(), Some(Arc::new(runtime)))
      }
    };

    Ok(Self {
      handle,
      _runtime: runtime,
      priority: Arc::new(RwLock::new(cfg.priority)),
      hybrid: Arc::new(AtomicBool::new(cfg.hybrid)),
      platform_listener: platform_listener.ok(),
      websocket_listener: websocket_listener.ok(),
    })
  }

  pub fn set_priority(&self, priority: MediaListenerPriority) {
    *self.priority.write().unwrap() = priority;
  }

  pub fn get_priority(&self) -> MediaListenerPriority {
    *self.priority.read().unwrap()
  }

  pub fn set_hybrid(&self, hybrid: bool) {
    self.hybrid.store(hybrid, Ordering::SeqCst)
  }

  pub fn toggle_hybrid(&self) {
    let _ = self
      .hybrid
      .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |value| Some(!value));
  }

  pub fn is_hybrid(&self) -> bool {
    self.hybrid.load(Ordering::SeqCst)
  }

  pub fn poll(&self) -> Result<MediaMetadata> {
    let priority = self.get_priority();
    let hybrid = self.is_hybrid();

    match (priority, &self.websocket_listener, &self.platform_listener) {
      (MediaListenerPriority::Websocket, Some(ws), Some(pl)) if hybrid => {
        let metadata = ws.poll();
        let fallback = pl.poll(true)?;

        let metadata = metadata_with_fallback(metadata, fallback);

        Ok(metadata)
      }
      (MediaListenerPriority::Websocket, Some(ws), _) => {
        let metadata = ws.poll();

        Ok(metadata)
      }
      (MediaListenerPriority::System, Some(ws), Some(pl)) if hybrid => {
        let metadata = pl.poll(true)?;
        let fallback = ws.poll();

        let metadata = metadata_with_fallback(metadata, fallback);

        Ok(metadata)
      }
      (MediaListenerPriority::System, _, Some(pl)) => pl.poll(true),
      _ => unreachable!(),
    }
  }

  pub fn poll_elapsed(&self) -> Result<Duration> {
    let priority = self.get_priority();
    let hybrid = self.is_hybrid();

    match (priority, &self.websocket_listener, &self.platform_listener) {
      (MediaListenerPriority::Websocket, Some(ws), Some(pl)) if hybrid => {
        let elapsed = ws.poll_elapsed();
        let fallback = pl.poll_elapsed(true)?;

        let elapsed = if elapsed == Duration::default() {
          fallback
        } else {
          elapsed
        };

        Ok(elapsed)
      }
      (MediaListenerPriority::Websocket, Some(ws), _) => {
        let elapsed = ws.poll_elapsed();

        Ok(elapsed)
      }
      (MediaListenerPriority::System, Some(ws), Some(pl)) if hybrid => {
        let elapsed = pl.poll_elapsed(true)?;
        let fallback = ws.poll_elapsed();

        let elapsed = if elapsed == Duration::default() {
          fallback
        } else {
          elapsed
        };

        Ok(elapsed)
      }
      (MediaListenerPriority::System, _, Some(pl)) => pl.poll_elapsed(true),
      _ => unreachable!(),
    }
  }
}

fn metadata_with_fallback(metadata: MediaMetadata, fallback: MediaMetadata) -> MediaMetadata {
  MediaMetadata {
    uid: metadata.uid.or(fallback.uid),
    uri: metadata.uri.or(fallback.uri),
    state: metadata.state,
    duration: if metadata.duration == Duration::default() {
      fallback.duration
    } else {
      metadata.duration
    },
    title: if metadata.title.is_empty() {
      fallback.title
    } else {
      metadata.title
    },
    album: metadata.album.or(fallback.album),
    artists: if metadata.artists.is_empty() {
      fallback.artists
    } else {
      metadata.artists
    },
    cover_url: metadata.cover_url.or(fallback.cover_url),
    cover: metadata.cover.or(fallback.cover),
    background_url: metadata.background_url.or(fallback.background_url),
    background: metadata.background.or(fallback.background),
  }
}
