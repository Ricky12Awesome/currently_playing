#![cfg(target_os = "linux")]

use std::sync::Arc;
use std::time::Duration;

use futures_locks::{Mutex, RwLock, RwLockReadGuard};
use mpris::{Metadata, PlaybackStatus, Player, PlayerFinder};
use thiserror::Error;
use tokio::runtime::{Handle as TokioHandle, Runtime as TokioRuntime};

use super::ForceSendSync;
use crate::{MediaMetadata, MediaState, Result};

#[derive(Error, Debug)]
#[error(transparent)]
pub enum MprisError {
  FindingError(#[from] mpris::FindingError),
  EventError(#[from] mpris::EventError),
  DBusError(#[from] mpris::DBusError),
  ProgressError(#[from] mpris::ProgressError),
  TrackListError(#[from] mpris::TrackListError),
}

#[derive(Debug)]
pub struct MediaListener {
  finder: Arc<ForceSendSync<PlayerFinder>>,
  handle: TokioHandle,
  _runtime: Option<Arc<TokioRuntime>>,
  player: Arc<RwLock<Option<ForceSendSync<Player>>>>,
  metadata: Arc<RwLock<Metadata>>,
  state: Arc<RwLock<MediaState>>,
  elapsed: Arc<RwLock<Duration>>,
}

impl MediaListener {
  /// Panics if it can't start d-bus session
  //noinspection DuplicatedCode
  #[allow(clippy::arc_with_non_send_sync)]
  pub fn new(handle: Option<TokioHandle>) -> Self {
    let finder = PlayerFinder::new().unwrap();
    let handle = handle.or_else(|| TokioHandle::try_current().ok());

    let (handle, runtime) = match handle {
      Some(handle) => (handle, None),
      None => {
        let runtime = TokioRuntime::new().unwrap();

        (runtime.handle().clone(), Some(Arc::new(runtime)))
      }
    };

    Self {
      finder: Arc::new(ForceSendSync(finder)),
      handle,
      _runtime: runtime,
      player: Arc::new(RwLock::new(None)),
      metadata: Arc::new(RwLock::new(Metadata::default())),
      state: Arc::new(RwLock::new(MediaState::Stopped)),
      elapsed: Arc::new(RwLock::new(Duration::default())),
    }
  }

  pub async fn new_async() -> Result<Self> {
    Ok(Self::new(Some(TokioHandle::current())))
  }

  async fn get_player(&self) -> RwLockReadGuard<Option<ForceSendSync<Player>>> {
    let guard = self.player.read().await;

    if let Some(player) = guard.as_ref() {
      if player.is_running() {
        return guard;
      }
    }

    let finder = self.finder.clone();
    let player = self.player.clone();

    self.handle.spawn(async move {
      let new_player = finder.find_active().ok();
      *player.write().await = new_player.map(ForceSendSync);
    });

    guard
  }

  pub async fn update_elapsed(&self) {
    let player = self.get_player().await;

    let Some(player) = player.as_ref() else {
      return;
    };

    let elapsed = player.get_position().unwrap();

    *self.elapsed.write().await = elapsed;
  }

  pub async fn update(&self) {
    let player = self.get_player().await;

    let Some(player) = player.as_ref() else {
      return;
    };

    let metadata = player.get_metadata().unwrap();
    let state = player.get_playback_status().unwrap();
    let elapsed = player.get_position().unwrap();

    *self.metadata.write().await = metadata;
    *self.state.write().await = state.into();
    *self.elapsed.write().await = elapsed;
  }

  pub fn poll(&self, update: bool) -> Result<MediaMetadata> {
    self.handle.block_on(self.poll_async(update))
  }

  pub async fn poll_async(&self, update: bool) -> Result<MediaMetadata> {
    if update {
      self.update().await;
    }

    let metadata = self.metadata.read().await;
    let state = self.state.read().await;

    Ok(MediaMetadata {
      uid: metadata.track_id().map(|s| s.to_string()),
      uri: metadata.url().map(|s| s.to_string()),
      state: *state,
      duration: metadata.length().unwrap_or_default(),
      title: metadata.title().unwrap_or_default().to_string(),
      album: metadata.album_name().map(|s| s.to_string()),
      artists: metadata
        .artists()
        .unwrap_or_default()
        .iter()
        .map(|s| s.to_string())
        .collect(),
      cover_url: metadata.art_url().map(|s| s.to_string()),
      cover: None,
      background_url: None,
      background: None,
    })
  }

  pub fn poll_elapsed(&self, update: bool) -> Result<Duration> {
    self.handle.block_on(self.poll_elapsed_async(update))
  }

  pub async fn poll_elapsed_async(&self, update: bool) -> Result<Duration> {
    if update {
      self.update_elapsed().await;
    }

    let elapsed = self.elapsed.read().await;

    Ok(*elapsed)
  }
}

impl From<PlaybackStatus> for MediaState {
  fn from(value: PlaybackStatus) -> Self {
    match value {
      PlaybackStatus::Playing => Self::Playing,
      PlaybackStatus::Paused => Self::Paused,
      PlaybackStatus::Stopped => Self::Stopped,
    }
  }
}
