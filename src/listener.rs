use std::net::SocketAddr;
use std::sync::{Arc, RwLock, RwLockReadGuard};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::platform::SystemMediaSource;
use crate::ws::WebsocketMediaSourceBackground;
use crate::{Error, MediaEvent, MediaMetadata, MediaState, Result};

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
  pub addr: WebsocketAddr,
  pub priority: MediaSourcePriority,
  pub timeout: Duration,
  pub update_rate: u64,
  pub hybrid: bool,
  pub websocket_enabled: bool,
  pub system_enabled: bool,
}

impl Default for MediaSourceConfig {
  fn default() -> Self {
    Self {
      addr: WebsocketAddr::Default,
      priority: MediaSourcePriority::Websocket,
      timeout: Duration::from_millis(5000),
      update_rate: 30,
      hybrid: true,
      websocket_enabled: true,
      system_enabled: true,
    }
  }
}

impl MediaSourceConfig {
  pub fn new() -> Self {
    Self {
      hybrid: false,
      websocket_enabled: false,
      system_enabled: false,
      ..Self::default()
    }
  }

  pub fn set_priority(self, priority: MediaSourcePriority) -> Self {
    Self { priority, ..self }
  }

  pub fn set_update_rate(self, update_rate: u64) -> Self {
    Self {
      update_rate,
      ..self
    }
  }

  pub fn set_hybrid(self, hybrid: bool) -> Self {
    Self { hybrid, ..self }
  }

  pub fn enable_system(self) -> Self {
    Self {
      system_enabled: true,
      ..self
    }
  }

  pub fn enable_websocket(self, addr: WebsocketAddr) -> Self {
    Self {
      addr,
      websocket_enabled: true,
      ..self
    }
  }
}

#[derive(Debug)]
pub struct MediaListener {
  system: Option<SystemMediaSource>,
  websocket: Option<WebsocketMediaSourceBackground>,
  last_played: Arc<RwLock<LastPlayed>>,
  cfg: MediaSourceConfig,
}

#[derive(Debug, Copy, Clone)]
enum LastPlayed {
  Websocket,
  System,
}

impl MediaSource for MediaListener {
  fn create(cfg: MediaSourceConfig) -> Result<Self> {
    if !cfg.system_enabled && !cfg.websocket_enabled {
      return Err(Error::NotEnabled);
    }

    let system = match cfg.system_enabled {
      true => {
        let source = SystemMediaSource::create(cfg.clone())?;
        Some(source)
      }
      false => None,
    };

    let websocket = match cfg.system_enabled {
      true => {
        let source = WebsocketMediaSourceBackground::create(cfg.clone())?;
        Some(source)
      }
      false => None,
    };

    let last_played = match cfg.priority {
      MediaSourcePriority::Websocket => LastPlayed::Websocket,
      MediaSourcePriority::System => LastPlayed::System,
    };

    let last_played = Arc::new(RwLock::new(last_played));

    Ok(Self {
      system,
      websocket,
      last_played,
      cfg,
    })
  }

  fn is_closed(&self) -> bool {
    let system = self
      .system
      .as_ref()
      .map(|s| s.is_closed())
      .unwrap_or_default();

    let websocket = self
      .websocket
      .as_ref()
      .map(|s| s.is_closed())
      .unwrap_or_default();

    system && websocket
  }

  fn is_running(&self) -> bool {
    let system = self
      .system
      .as_ref()
      .map(|s| s.is_running())
      .unwrap_or_default();

    let websocket = self
      .websocket
      .as_ref()
      .map(|s| s.is_running())
      .unwrap_or_default();

    system || websocket
  }

  fn poll(&self) -> Result<MediaMetadata> {
    match (self.cfg.priority, &self.system, &self.websocket) {
      (MediaSourcePriority::System, Some(system), Some(websocket)) => {
        let system = system.poll()?;
        let websocket = websocket.poll()?;

        match (system.state, websocket.state) {
          (MediaState::Playing, MediaState::Playing) => {
            *self.last_played.write().unwrap() = LastPlayed::System;
            Ok(system)
          },
          (MediaState::Stopped | MediaState::Paused, MediaState::Playing) => {
            *self.last_played.write().unwrap() = LastPlayed::Websocket;
            Ok(websocket)
          },
          (MediaState::Playing, MediaState::Stopped | MediaState::Paused) => {
            *self.last_played.write().unwrap() = LastPlayed::System;
            Ok(system)
          },
          _ => match *self.last_played.read().unwrap() {
            LastPlayed::Websocket => Ok(websocket),
            LastPlayed::System => Ok(system),
          }
        }
      }
      (MediaSourcePriority::System, Some(system), None) => system.poll(),
      (MediaSourcePriority::System, None, Some(websocket)) => websocket.poll(),
      (MediaSourcePriority::Websocket, Some(system), Some(websocket)) => {
        let system = system.poll()?;
        let websocket = websocket.poll()?;

        match (system.state, websocket.state) {
          (MediaState::Playing, MediaState::Playing) => {
            *self.last_played.write().unwrap() = LastPlayed::Websocket;
            Ok(websocket)
          },
          (MediaState::Playing, MediaState::Stopped | MediaState::Paused) => {
            *self.last_played.write().unwrap() = LastPlayed::System;
            Ok(system)
          },
          (MediaState::Stopped | MediaState::Paused, MediaState::Playing) => {
            *self.last_played.write().unwrap() = LastPlayed::Websocket;
            Ok(websocket)
          },
          _ => match *self.last_played.read().unwrap() {
            LastPlayed::Websocket => Ok(websocket),
            LastPlayed::System => Ok(system),
          }
        }
      }
      (MediaSourcePriority::Websocket, None, Some(websocket)) => websocket.poll(),
      (MediaSourcePriority::Websocket, Some(system), None) => system.poll(),
      _ => unreachable!(),
    }
  }

  fn poll_guarded(&self) -> Result<RwLockReadGuard<MediaMetadata>> {
    match (self.cfg.priority, &self.system, &self.websocket) {
      (MediaSourcePriority::System, Some(system), Some(websocket)) => {
        let system = system.poll_guarded()?;
        let websocket = websocket.poll_guarded()?;

        match (system.state, websocket.state) {
          (MediaState::Playing, MediaState::Playing) => {
            *self.last_played.write().unwrap() = LastPlayed::System;
            Ok(system)
          },
          (MediaState::Stopped | MediaState::Paused, MediaState::Playing) => {
            *self.last_played.write().unwrap() = LastPlayed::Websocket;
            Ok(websocket)
          },
          (MediaState::Playing, MediaState::Stopped | MediaState::Paused) => {
            *self.last_played.write().unwrap() = LastPlayed::System;
            Ok(system)
          },
          _ => match *self.last_played.read().unwrap() {
            LastPlayed::Websocket => Ok(websocket),
            LastPlayed::System => Ok(system),
          }
        }
      }
      (MediaSourcePriority::System, Some(system), None) => system.poll_guarded(),
      (MediaSourcePriority::System, None, Some(websocket)) => websocket.poll_guarded(),
      (MediaSourcePriority::Websocket, Some(system), Some(websocket)) => {
        let system = system.poll_guarded()?;
        let websocket = websocket.poll_guarded()?;

        match (system.state, websocket.state) {
          (MediaState::Playing, MediaState::Playing) => {
            *self.last_played.write().unwrap() = LastPlayed::System;
            Ok(system)
          },
          (MediaState::Playing, MediaState::Stopped | MediaState::Paused) => {
            *self.last_played.write().unwrap() = LastPlayed::System;
            Ok(system)
          },
          (MediaState::Stopped | MediaState::Paused, MediaState::Playing) => {
            *self.last_played.write().unwrap() = LastPlayed::Websocket;
            Ok(websocket)
          },
          _ => match *self.last_played.read().unwrap() {
            LastPlayed::Websocket => Ok(websocket),
            LastPlayed::System => Ok(system),
          }
        }
      }
      (MediaSourcePriority::Websocket, None, Some(websocket)) => websocket.poll_guarded(),
      (MediaSourcePriority::Websocket, Some(system), None) => system.poll_guarded(),
      _ => unreachable!(),
    }
  }

  fn next(&self) -> Result<MediaEvent> {
    match (self.cfg.priority, &self.system, &self.websocket) {
      (MediaSourcePriority::System, Some(system), Some(websocket)) => {
        system.next().or_else(|_| websocket.next())
      }
      (MediaSourcePriority::System, Some(system), None) => system.next(),
      (MediaSourcePriority::System, None, Some(websocket)) => websocket.next(),
      (MediaSourcePriority::Websocket, Some(system), Some(websocket)) => {
        websocket.next().or_else(|_| system.next())
      }
      (MediaSourcePriority::Websocket, None, Some(websocket)) => websocket.next(),
      (MediaSourcePriority::Websocket, Some(system), None) => system.next(),
      _ => unreachable!(),
    }
  }
}

pub trait MediaSource: Send + Sync + Sized {
  fn create(cfg: MediaSourceConfig) -> Result<Self>;

  fn is_closed(&self) -> bool;

  fn is_running(&self) -> bool;

  fn poll(&self) -> Result<MediaMetadata>;

  fn poll_guarded(&self) -> Result<RwLockReadGuard<MediaMetadata>>;

  fn next(&self) -> Result<MediaEvent>;
}
