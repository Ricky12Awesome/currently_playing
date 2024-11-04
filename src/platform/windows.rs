#![cfg(windows)]

use crate::listener::{MediaSource, MediaSourceConfig};
use crate::{Error, MediaEvent, MediaImage, MediaMetadata, MediaState, Result};
use std::fmt::Debug;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender};
use std::sync::{Arc, Mutex, RwLock, RwLockReadGuard};
use std::thread::JoinHandle;
use std::time::Duration;
use windows::Foundation::TypedEventHandler;
use windows::Media::Control::{
  CurrentSessionChangedEventArgs, GlobalSystemMediaTransportControlsSession,
  GlobalSystemMediaTransportControlsSessionManager,
  GlobalSystemMediaTransportControlsSessionPlaybackStatus, TimelinePropertiesChangedEventArgs,
};
use windows::Storage::Streams::DataReader;

//noinspection DuplicatedCode
#[derive(Debug)]
pub struct WindowsMediaSource {
  timeout: Duration,
  cancel_token: Arc<AtomicBool>,
  is_running: Arc<AtomicBool>,
  metadata: Arc<RwLock<MediaMetadata>>,
  recv: Arc<Mutex<Receiver<MediaEvent>>>,
  _background_task: JoinHandle<()>,
}

//noinspection DuplicatedCode
impl MediaSource for WindowsMediaSource {
  fn create(cfg: MediaSourceConfig) -> Result<Self> {
    if !cfg.system_enabled {
      return Err(Error::NotEnabled);
    }

    let update_rate = cfg.update_rate;
    let cancel_token = Arc::new(AtomicBool::new(false));
    let is_running = Arc::new(AtomicBool::new(false));
    let metadata = Arc::new(RwLock::new(MediaMetadata::default()));
    let (send, recv) = std::sync::mpsc::sync_channel(0);

    let _background_task = spawn_background_task(
      update_rate,
      cancel_token.clone(),
      is_running.clone(),
      metadata.clone(),
      send,
    );

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

//noinspection DuplicatedCode
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
        std::thread::sleep(Duration::from_millis(100));
        continue;
      }
    }
  })
}

//noinspection DuplicatedCode
fn background_task(
  update_rate: u64,
  cancel_token: Arc<AtomicBool>,
  is_running: Arc<AtomicBool>,
  metadata_handle: Arc<RwLock<MediaMetadata>>,
  send: SyncSender<MediaEvent>,
) -> Result<()> {
  let manager = GlobalSystemMediaTransportControlsSessionManager::RequestAsync()?.get()?;

  
  let wait_ms = 1000u64.checked_div(update_rate).unwrap_or(1);
  let wait = Duration::from_millis(wait_ms);

  // let session = Arc::new(RwLock::new(session));
  // 
  // let session_handle = session.clone();
  // let event = TypedEventHandler::<
  //   GlobalSystemMediaTransportControlsSessionManager,
  //   CurrentSessionChangedEventArgs,
  // >::new(move |manager, _| {
  //   let Some(manager) = manager else {
  //     return Ok(());
  //   };
  // 
  //   let Ok(new_session) = manager.GetCurrentSession() else {
  //     return Ok(());
  //   };
  // 
  //   {
  //     *session_handle.write().unwrap() = new_session;
  //   }
  // 
  //   Ok(())
  // });
  // 
  // manager.CurrentSessionChanged(&event)?;

  loop {
    if cancel_token.load(Ordering::SeqCst) {
      break;
    }
    
    let session = manager.GetCurrentSession()?;

    // let session = session.read().unwrap();

    is_running.store(true, Ordering::SeqCst);

    let metadata = metadata_handle.read().unwrap();

    let timeline = session.GetTimelineProperties()?;
    let info = session.GetPlaybackInfo()?;
    let props = session.TryGetMediaPropertiesAsync()?.get()?;

    let state = info.PlaybackStatus()?.into();
    let elapsed = timeline.Position()?.into();

    let thumbnail = props.Thumbnail();
    let thumbnail = thumbnail?.OpenReadAsync()?.get()?;
    let size = thumbnail.Size()?;
    let pos = thumbnail.Position()?;
    let stream = thumbnail.GetInputStreamAt(pos)?;
    let reader = DataReader::CreateDataReader(&stream)?;

    let mut buf = vec![0u8; size as _];

    reader.LoadAsync(size as _)?.get()?;
    reader.ReadBytes(&mut buf)?;

    thumbnail.Close()?;

    let thumbnail = MediaImage {
      format: thumbnail.ContentType()?.to_string_lossy().into(),
      data: buf,
    };

    let new_metadata = MediaMetadata {
      uid: None,
      uri: None,
      state,
      duration: timeline.EndTime()?.into(),
      elapsed,
      title: props.Title()?.to_string_lossy(),
      album: props.AlbumTitle().ok().map(|s| s.to_string_lossy()),
      artists: props
        .Artist()
        .map(|s| s.to_string_lossy())
        .into_iter()
        .collect(),
      cover_url: None,
      cover: Some(thumbnail),
      background_url: None,
      background: None,
    };

    let event = match () {
      _ if metadata.is_different(&new_metadata) => {
        Some(MediaEvent::MediaChanged(new_metadata.clone()))
      }
      _ if metadata.state != state => Some(MediaEvent::StateChanged(state)),
      _ if state == MediaState::Playing => Some(MediaEvent::ProgressChanged(elapsed)),
      _ => None,
    };

    drop(metadata);

    let mut metadata = metadata_handle.write().unwrap();

    *metadata = new_metadata;

    drop(metadata);

    if let Some(event) = event {
      let _ = send.try_send(event);
    }

    std::thread::sleep(wait);
  }

  Ok(())
}

impl From<GlobalSystemMediaTransportControlsSessionPlaybackStatus> for MediaState {
  fn from(value: GlobalSystemMediaTransportControlsSessionPlaybackStatus) -> Self {
    use GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status;

    match value {
      Status::Stopped | Status::Closed | Status::Opened => Self::Stopped,
      Status::Paused | Status::Changing => Self::Paused,
      Status::Playing => Self::Playing,
      _ => unreachable!(),
    }
  }
}
