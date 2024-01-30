use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

pub type PlatformListener = crate::platform::MediaListener;
pub type WebsocketListener = crate::ws::MediaListener;
pub type WebsocketListenerPooled = crate::ws::MediaListenerPooled;

use crate::{Error, MediaEvent, MediaMetadata, Result, TokioHandle, TokioRuntime};

#[derive(Debug)]
#[allow(unused)]
pub struct MediaListenerImpl {
  handle: TokioHandle,
  _runtime: Option<Arc<TokioRuntime>>,
  priority: Arc<RwLock<MediaSourcePriority>>,
  hybrid: Arc<AtomicBool>,
  platform_listener: Option<PlatformListener>,
  websocket_listener: Option<WebsocketListenerPooled>,
}

#[derive(
  Default, Debug, Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Hash, Serialize, Deserialize,
)]
pub enum WebsocketAddr {
  Local(u16),
  Addr(SocketAddr),

  #[default]
  Default,
}

#[derive(
  Default, Debug, Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Hash, Serialize, Deserialize,
)]
pub enum MediaSourcePriority {
  #[default]
  Websocket,
  System,
}

#[derive(Debug, Clone)]
pub struct MediaSourceConfig {
  pub handle: Option<TokioHandle>,
  pub addr: WebsocketAddr,
  pub priority: MediaSourcePriority,
  pub update_rate: usize,
  pub hybrid: bool,
  pub websocket_enabled: bool,
  pub system_enabled: bool,
}

impl Default for MediaSourceConfig {
  fn default() -> Self {
    Self {
      handle: None,
      addr: WebsocketAddr::Default,
      priority: MediaSourcePriority::Websocket,
      update_rate: 30,
      hybrid: true,
      websocket_enabled: true,
      system_enabled: true,
    }
  }
}

impl MediaSourceConfig {
  fn new(handle: Option<TokioHandle>) -> Self {
    Self {
      handle,
      hybrid: false,
      websocket_enabled: false,
      system_enabled: false,
      ..Self::default()
    }
  }

  fn set_handle(self, handle: Option<TokioHandle>) -> Self {
    Self {
      handle,
      ..self
    }
  }

  fn set_priority(self, priority: MediaSourcePriority) -> Self {
    Self {
      priority,
      ..self
    }
  }

  fn set_update_rate(self, update_rate: usize) -> Self {
    Self {
      update_rate,
      ..self
    }
  }

  fn set_hybrid(self, hybrid: bool) -> Self {
    Self {
      hybrid,
      ..self
    }
  }

  fn enable_system(self) -> Self {
    Self {
      system_enabled: true,
      ..self
    }
  }

  fn enable_websocket(self, addr: WebsocketAddr) -> Self {
    Self {
      addr,
      websocket_enabled: true,
      ..self
    }
  }
}

impl MediaListenerImpl {
  //noinspection DuplicatedCode
  pub fn new(cfg: MediaSourceConfig) -> Result<Self> {
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
  pub async fn new_async(cfg: MediaSourceConfig) -> Result<Self> {
    let websocket_listener = match cfg.addr {
      WebsocketAddr::Local(port) => WebsocketListener::bind_local(port).await,
      WebsocketAddr::Addr(addr) => WebsocketListener::bind(addr).await,
      WebsocketAddr::Default => WebsocketListener::bind_default().await,
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

  pub fn set_priority(&self, priority: MediaSourcePriority) {
    *self.priority.write().unwrap() = priority;
  }

  pub fn get_priority(&self) -> MediaSourcePriority {
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
      (MediaSourcePriority::Websocket, Some(ws), Some(pl)) if hybrid => {
        let metadata = ws.poll();
        let fallback = pl.poll(true)?;

        let metadata = metadata.merge(fallback);

        Ok(metadata)
      }
      (MediaSourcePriority::Websocket, Some(ws), _) => {
        let metadata = ws.poll();

        Ok(metadata)
      }
      (MediaSourcePriority::System, Some(ws), Some(pl)) if hybrid => {
        let metadata = pl.poll(true)?;
        let fallback = ws.poll();

        let metadata = metadata.merge(fallback);

        Ok(metadata)
      }
      (MediaSourcePriority::System, _, Some(pl)) => pl.poll(true),
      _ => unreachable!(),
    }
  }

  pub fn poll_elapsed(&self) -> Result<Duration> {
    let priority = self.get_priority();
    let hybrid = self.is_hybrid();

    match (priority, &self.websocket_listener, &self.platform_listener) {
      (MediaSourcePriority::Websocket, Some(ws), Some(pl)) if hybrid => {
        let elapsed = ws.poll_elapsed();
        let fallback = pl.poll_elapsed(true)?;

        let elapsed = if elapsed == Duration::default() {
          fallback
        } else {
          elapsed
        };

        Ok(elapsed)
      }
      (MediaSourcePriority::Websocket, Some(ws), _) => {
        let elapsed = ws.poll_elapsed();

        Ok(elapsed)
      }
      (MediaSourcePriority::System, Some(ws), Some(pl)) if hybrid => {
        let elapsed = pl.poll_elapsed(true)?;
        let fallback = ws.poll_elapsed();

        let elapsed = if elapsed == Duration::default() {
          fallback
        } else {
          elapsed
        };

        Ok(elapsed)
      }
      (MediaSourcePriority::System, _, Some(pl)) => pl.poll_elapsed(true),
      _ => unreachable!(),
    }
  }
}


pub trait MediaSource: Send + Sync {
  fn create(cfg: MediaSourceConfig) -> Self;

  fn poll(&self) -> Result<MediaMetadata>;

  fn next(&self) -> Result<MediaEvent>;
}