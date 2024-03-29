#![cfg(target_os = "linux")]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender};
use std::sync::{Arc, Mutex, RwLock, RwLockReadGuard};
use std::thread::JoinHandle;
use std::time::Duration;

use mpris::{PlaybackStatus, PlayerFinder};

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
  timeout: Duration,
  cancel_token: Arc<AtomicBool>,
  is_running: Arc<AtomicBool>,
  metadata: Arc<RwLock<MediaMetadata>>,
  recv: Arc<Mutex<Receiver<MediaEvent>>>,
  _background_task: JoinHandle<()>,
}

impl MediaSource for MprisMediaSource {
  fn create(cfg: MediaSourceConfig) -> Result<Self> {
    if !cfg.system_enabled {
      return Err(Error::NotEnabled);
    }

    let update_rate = cfg.update_rate;
    let cancel_token = Arc::new(AtomicBool::new(false));
    let is_running = Arc::new(AtomicBool::new(false));
    let metadata = Arc::new(RwLock::new(MediaMetadata::default()));
    let (send, recv) = std::sync::mpsc::sync_channel(0);

    let _background_task =
      spawn_background_task(update_rate, cancel_token.clone(), is_running.clone(), metadata.clone(), send);

    let recv = Arc::new(Mutex::new(recv));

    Ok(Self {
      timeout: cfg.timeout,
      cancel_token,
      is_running,
      metadata,
      recv,
      _background_task,
    })
  }

  fn is_closed(&self) -> bool {
    self.cancel_token.load(Ordering::SeqCst)
  }

  fn is_running(&self) -> bool {
    self.is_running.load(Ordering::SeqCst)
  }

  fn poll(&self) -> Result<MediaMetadata> {
    self.poll_guarded().map(|v| v.clone())
  }

  fn poll_guarded(&self) -> Result<RwLockReadGuard<MediaMetadata>> {
    if self.is_closed() {
      return Err(Error::Closed);
    }

    Ok(self.metadata.read().unwrap())
  }

  fn next(&self) -> Result<MediaEvent> {
    if self.is_closed() {
      return Err(Error::Closed);
    }

    let recv = self.recv.lock().unwrap();
    let event = recv.recv_timeout(self.timeout)?;

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
  is_running: Arc<AtomicBool>,
  metadata: Arc<RwLock<MediaMetadata>>,
  send: SyncSender<MediaEvent>,
) -> JoinHandle<()> {
  std::thread::spawn(move || loop {
    let result = background_task(
      update_rate,
      cancel_token.clone(),
      is_running.clone(),
      metadata.clone(),
      send.clone(),
    );

    match result {
      Ok(_) => break,
      Err(_) => {
        is_running.store(false, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(1000));
        continue;
      }
    }
  })
}

#[allow(clippy::await_holding_lock)]
fn background_task(
  update_rate: u64,
  cancel_token: Arc<AtomicBool>,
  is_running: Arc<AtomicBool>,
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

    is_running.store(player.is_running(), Ordering::SeqCst);

    if !player.is_running() {
      player = finder.find_active().map_err(MprisError::from)?;

      if !player.is_running() {
        std::thread::sleep(Duration::from_millis(1000));
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
      _ if state == MediaState::Playing => Some(MediaEvent::ProgressChanged(elapsed)),
      _ => None,
    };

    *metadata = new_metadata;
    drop(metadata);

    if let Some(event) = event {
      let _ = send.try_send(event);
    }

    std::thread::sleep(wait);
  }

  Ok(())
}
