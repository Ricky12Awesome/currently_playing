#![cfg(target_os = "linux")]

use mpris::{Metadata, PlaybackStatus, PlayerFinder};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender};
use std::sync::{Arc, Mutex, RwLock};
use std::thread::JoinHandle;
use std::time::Duration;
use tokio::runtime::{Builder, Runtime};

use crate::listener::{MediaSource, MediaSourceConfig};
use crate::{Error, MediaEvent, MediaMetadata, MediaState, Result};

#[derive(thiserror::Error, Debug)]
#[error(transparent)]
pub enum MprisError {
  FindingError(#[from] mpris::FindingError),
  EventError(#[from] mpris::EventError),
  DBusError(#[from] mpris::DBusError),
  ProgressError(#[from] mpris::ProgressError),
  TrackListError(#[from] mpris::TrackListError),
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

#[derive(Debug)]
pub struct MprisMediaSource {
  cancel_token: Arc<AtomicBool>,
  metadata: Arc<RwLock<MediaMetadata>>,
  recv: Arc<Mutex<Receiver<MediaEvent>>>,
  _background_task: JoinHandle<()>,
}

impl MprisMediaSource {
  pub fn is_closed(&self) -> bool {
    self.cancel_token.load(Ordering::SeqCst)
  }
}

impl MediaSource for MprisMediaSource {
  fn create(cfg: MediaSourceConfig) -> Result<Self> {
    if !cfg.system_enabled {
      return Err(Error::NotEnabled);
    }

    let update_rate = cfg.update_rate;
    let cancel_token = Arc::new(AtomicBool::new(false));
    let metadata = Arc::new(RwLock::new(MediaMetadata::default()));
    let (send, recv) = std::sync::mpsc::sync_channel(0);

    let _background_task = spawn_background_task(
      update_rate,
      cancel_token.clone(),
      metadata.clone(),
      send
    );

    let recv = Arc::new(Mutex::new(recv));

    Ok(Self {
      cancel_token,
      metadata,
      recv,
      _background_task,
    })
  }

  fn poll(&self) -> Result<MediaMetadata> {
    if self.is_closed() {
      return Err(Error::Closed)
    }

    Ok(self.metadata.read().unwrap().clone())
  }

  fn next(&self) -> Result<MediaEvent> {
    if self.is_closed() {
      return Err(Error::Closed)
    }

    let timeout = Duration::from_millis(1000);
    let recv = self.recv.lock().unwrap();
    let event = recv.recv_timeout(timeout)?;

    Ok(event)
  }
}

impl Drop for MprisMediaSource {
  fn drop(&mut self) {
    self.cancel_token.store(true, Ordering::SeqCst)
  }
}


fn spawn_background_task(
  update_rate: u64,
  cancel_token: Arc<AtomicBool>,
  metadata: Arc<RwLock<MediaMetadata>>,
  send: SyncSender<MediaEvent>,
) -> JoinHandle<()> {
  std::thread::spawn(move || {
    let runtime = Builder::new_multi_thread()
      .worker_threads(2)
      .enable_all()
      .build()
      .unwrap();

    loop {
      let task = background_task(
        update_rate,
        cancel_token.clone(),
        metadata.clone(),
        send.clone(),
      );

      let result = runtime.block_on(task);

      match result {
        Ok(_) => break,
        Err(_) => {
          std::thread::sleep(Duration::from_millis(1000));
          continue;
        }
      }
    }
  })
}

#[allow(clippy::await_holding_lock)]
async fn background_task(
  update_rate: u64,
  cancel_token: Arc<AtomicBool>,
  metadata: Arc<RwLock<MediaMetadata>>,
  send: SyncSender<MediaEvent>,
) -> Result<()> {
  let finder = PlayerFinder::new().map_err(MprisError::from)?;
  let mut player = finder.find_active().map_err(MprisError::from)?;

  let wait_ms = 1000u64.checked_div(update_rate).unwrap_or(1);
  let wait = Duration::from_millis(wait_ms);

  loop {
    if cancel_token.load(Ordering::SeqCst) {
      break;
    }

    if !player.is_running() {
      player = finder.find_active().map_err(MprisError::from)?;

      if !player.is_running() {
        tokio::time::sleep(Duration::from_millis(1000)).await;
        continue;
      }
    };

    let mpris_metadata = player.get_metadata().map_err(MprisError::from)?;
    let elapsed = player.get_position().map_err(MprisError::from)?;
    let state = player
      .get_playback_status()
      .map(MediaState::from)
      .map_err(MprisError::from)?;

    let new_metadata = MediaMetadata {
      uid: mpris_metadata.track_id().map(Into::into),
      uri: mpris_metadata.url().map(Into::into),
      state,
      duration: mpris_metadata.length().unwrap_or_default(),
      elapsed,
      title: mpris_metadata.title().map(Into::into).unwrap_or_default(),
      album: mpris_metadata.album_name().map(Into::into),
      artists: mpris_metadata
        .artists()
        .unwrap_or_default()
        .iter()
        .map(|s| s.to_string())
        .collect(),
      cover_url: mpris_metadata.art_url().map(Into::into),
      cover: None,
      background_url: None,
      background: None,
    };

    let mut metadata = metadata.write().unwrap();

    let event = match () {
      _ if metadata.is_different(&new_metadata) => {
        Some(MediaEvent::MediaChanged(new_metadata.clone()))
      }
      _ if metadata.state != state => Some(MediaEvent::StateChanged(state)),
      _ if state == MediaState::Playing => Some(MediaEvent::ProgressChanged(
        elapsed.as_secs_f64() / metadata.duration.as_secs_f64(),
      )),
      _ => None,
    };

    *metadata = new_metadata;
    drop(metadata);

    if let Some(event) = event {
      let _ = send.try_send(event);
    }

    tokio::time::sleep(wait).await;
  }

  Ok(())
}
