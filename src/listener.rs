use std::net::SocketAddr;

use serde::{Deserialize, Serialize};

use crate::{MediaEvent, MediaMetadata, Result};

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

pub struct MediaListener {

}

pub trait MediaSource: Send + Sync + Sized {
  fn create(cfg: MediaSourceConfig) -> Result<Self>;

  fn poll(&self) -> Result<MediaMetadata>;

  fn next(&self) -> Result<MediaEvent>;
}
