#![cfg(windows)]

use std::fmt::{Debug, Formatter};
use std::ops::Deref;
use std::sync::Arc;
use std::time::Duration;

use futures_locks::RwLock;
use tokio::runtime::{Handle as TokioHandle, Runtime as TokioRuntime};
use tokio::task::JoinHandle;
use windows::core::Error as WError;
use windows::core::{Result as WResult, HRESULT};
use windows::Foundation::TypedEventHandler;
use windows::Media::Control::{
  CurrentSessionChangedEventArgs, GlobalSystemMediaTransportControlsSession,
  GlobalSystemMediaTransportControlsSessionManager,
  GlobalSystemMediaTransportControlsSessionPlaybackStatus, MediaPropertiesChangedEventArgs,
  PlaybackInfoChangedEventArgs, TimelinePropertiesChangedEventArgs,
};
use windows::Storage::Streams::DataReader;

use super::ForceSendSync;
use crate::{MediaImage, MediaMetadata, MediaState, Result};

pub type TimelinePropertiesChangedEvent =
  TypedEventHandler<GlobalSystemMediaTransportControlsSession, TimelinePropertiesChangedEventArgs>;
pub type PlaybackInfoChangedEvent =
  TypedEventHandler<GlobalSystemMediaTransportControlsSession, PlaybackInfoChangedEventArgs>;

pub type MediaPropertiesChangedEvent =
  TypedEventHandler<GlobalSystemMediaTransportControlsSession, MediaPropertiesChangedEventArgs>;

pub type SessionChangedEvent = TypedEventHandler<
  GlobalSystemMediaTransportControlsSessionManager,
  CurrentSessionChangedEventArgs,
>;

#[derive(Debug)]
pub struct MediaListener {
  manager: Arc<RwLock<Option<ForceSendSync<GlobalSystemMediaTransportControlsSessionManager>>>>,
  session: Arc<RwLock<WResult<SessionManager>>>,
  handle: TokioHandle,
  _runtime: Option<Arc<TokioRuntime>>,
  _background: Option<JoinHandle<Result<()>>>,
}

impl MediaListener {
  pub async fn new_with_handle_async(handle: TokioHandle) -> Result<Self> {
    let manager = GlobalSystemMediaTransportControlsSessionManager::RequestAsync()?.await?;
    let session = Arc::new(RwLock::new(
      manager
        .GetCurrentSession()
        .map(|s| SessionManager::new(s, handle.clone())),
    ));

    let manager = Arc::new(RwLock::new(Some(ForceSendSync(manager))));

    Self::setup_events(manager.clone(), session.clone(), handle.clone()).await?;

    let this = Self {
      session,
      _runtime: None,
      _background: None,
      handle,
      manager,
    };

    Ok(this)
  }

  /// # Panics
  ///
  /// This will panic if called outside the context of a Tokio runtime. ([tokio::runtime::Handle::current])
  ///
  /// use [MediaListener::new_with_handle_async] instead
  pub async fn new_async() -> Result<Self> {
    Self::new_with_handle_async(TokioHandle::current()).await
  }

  /// Creates a MediaListener that updates in the background
  ///
  /// so you don't have to deal with async/await
  //noinspection DuplicatedCode
  pub fn new(handle: Option<TokioHandle>) -> Self {
    let handle = handle.or_else(|| TokioHandle::try_current().ok());

    let (handle, runtime) = match handle {
      Some(handle) => (handle, None),
      None => {
        let runtime = TokioRuntime::new().unwrap();

        (runtime.handle().clone(), Some(Arc::new(runtime)))
      }
    };

    let mut this = Self {
      manager: Arc::new(RwLock::new(None)),
      session: Arc::new(RwLock::new(Err(WError::new(
        HRESULT(0),
        "undefined".into(),
      )))),
      handle: handle.clone(),
      _runtime: runtime,
      _background: None,
    };

    let this_manager = this.manager.clone();
    let this_session = this.session.clone();
    let this_handle = this.handle.clone();

    let background = handle.spawn(async move {
      let manager = GlobalSystemMediaTransportControlsSessionManager::RequestAsync()?.await?;

      *this_session.write().await = manager
        .GetCurrentSession()
        .map(|s| SessionManager::new(s, this_handle.clone()));

      *this_manager.write().await = Some(ForceSendSync(manager));

      Self::setup_events(this_manager, this_session, this_handle).await?;

      Ok(())
    });

    while !background.is_finished() {
      std::thread::sleep(Duration::from_millis(1));
    }

    this._background = Some(background);

    this
  }

  pub fn poll_elapsed(&self, update: bool) -> Result<Duration> {
    self.handle.block_on(self.poll_elapsed_async(update))
  }

  pub async fn poll_elapsed_async(&self, update: bool) -> Result<Duration> {
    // stupid lock guard doesn't let be use Try ?
    let session = self.session.read().await;
    let session = session.as_ref();

    if let Err(err) = session {
      return Err(err.clone().into());
    }

    let session = session.unwrap();

    if update {
      session.update_elapsed().await?;
    }

    let elapsed = session.get_elapsed().await;

    Ok(elapsed)
  }

  pub fn poll(&self, update: bool) -> Result<MediaMetadata> {
    self.handle.block_on(self.poll_async(update))
  }

  pub async fn poll_async(&self, update: bool) -> Result<MediaMetadata> {
    // stupid lock guard doesn't let be use Try ?
    let session = self.session.read().await;
    let session = session.as_ref();

    if let Err(err) = session {
      return Err(err.clone().into());
    }

    let session = session.unwrap();

    if update {
      session.update().await;
    }

    let metadata = session.to_metadata().await;

    Ok(metadata)
  }

  async fn setup_events(
    manager: Arc<RwLock<Option<ForceSendSync<GlobalSystemMediaTransportControlsSessionManager>>>>,
    handle: Arc<RwLock<WResult<SessionManager>>>,
    tokio: TokioHandle,
  ) -> Result<()> {
    let event = SessionChangedEvent::new(move |manager, _| {
      let Some(manager) = manager else {
        return Ok(());
      };

      let Ok(new_session) = manager.GetCurrentSession() else {
        return Ok(());
      };

      tokio.block_on(async {
        let mut session = handle.write().await;

        let Ok(session) = session.as_mut() else {
          return Ok(());
        };

        session.update_session(new_session).await;

        Ok(())
      })
    });

    let event = ForceSendSync(event);
    let manager = manager.read().await;
    let manager = manager.as_ref().unwrap();

    manager.CurrentSessionChanged(&event.0)?;

    Ok(())
  }
}

#[derive(Debug)]
struct SessionManager {
  session: Arc<RwLock<GlobalSystemMediaTransportControlsSession>>,
  handle: TokioHandle,
  // not Option<String> since there is almost always a title,
  // if there is not just default to "" for nothing
  title: Arc<RwLock<String>>,
  artist: Arc<RwLock<String>>,
  duration: Arc<RwLock<Duration>>,
  elapsed: Arc<RwLock<Duration>>,
  state: Arc<RwLock<MediaState>>,
  thumbnail: Arc<RwLock<Option<MediaImage>>>,
}

unsafe impl Send for SessionManager {}
unsafe impl Sync for SessionManager {}

impl SessionManager {
  fn new(session: GlobalSystemMediaTransportControlsSession, handle: TokioHandle) -> Self {
    Self {
      session: Arc::new(RwLock::new(session)),
      handle,
      title: Arc::new(RwLock::new("".into())),
      artist: Arc::new(RwLock::new("".into())),
      duration: Arc::new(RwLock::new(Duration::default())),
      elapsed: Arc::new(RwLock::new(Duration::default())),
      state: Arc::new(RwLock::new(MediaState::Stopped)),
      thumbnail: Arc::new(RwLock::new(None)),
    }
  }

  pub async fn update_elapsed(&self) -> Result<()> {
    let session = self.session.clone();
    let _elapsed = self.elapsed.clone();

    let session = session.read().await;
    let timeline = session.GetTimelineProperties()?;
    let elapsed = timeline.Position()?;

    *_elapsed.write().await = elapsed.into();

    Ok(())
  }

  pub async fn update(&self) -> [JoinHandle<Result<()>>; 2] {
    let prop_session = self.session.clone();

    let _title = self.title.clone();
    let _artist = self.artist.clone();
    let _duration = self.duration.clone();
    let _elapsed = self.elapsed.clone();
    let _state = self.state.clone();

    let properties: JoinHandle<Result<()>> = self.handle.spawn(async move {
      let session = prop_session.read().await;
      let timeline = session.GetTimelineProperties()?;
      let info = session.GetPlaybackInfo()?;
      let props = session.TryGetMediaPropertiesAsync()?.await?;
      let title = props.Title().map(|s| s.to_string_lossy());
      let artist = props.Artist().map(|s| s.to_string_lossy());
      let state = info.PlaybackStatus().map(MediaState::from);
      let duration = timeline.EndTime()?;
      let elapsed = timeline.Position()?;

      *_title.write().await = title.unwrap_or("".into());
      *_artist.write().await = artist.unwrap_or("".into());
      *_duration.write().await = duration.into();
      *_elapsed.write().await = elapsed.into();
      *_state.write().await = state.unwrap_or(MediaState::Stopped);
      Ok(())
    });

    let thumbnail_session = self.session.clone();
    let _thumbnail = self.thumbnail.clone();

    let thumbnail: JoinHandle<Result<()>> = self.handle.spawn(async move {
      let session = thumbnail_session.read().await;
      let props = session.TryGetMediaPropertiesAsync()?.await?;

      let thumbnail = ForceSendSync(props.Thumbnail()?);
      let thumbnail = ForceSendSync(thumbnail.OpenReadAsync()?.await?);
      let size = thumbnail.Size()?;
      let pos = thumbnail.Position()?;
      let stream = ForceSendSync(thumbnail.GetInputStreamAt(pos)?);
      let reader = ForceSendSync(DataReader::CreateDataReader(&stream.0)?);

      let mut buf = vec![0u8; size as _];

      reader.LoadAsync(size as _)?.await?;
      reader.ReadBytes(&mut buf)?;

      thumbnail.Close()?;

      let thumbnail = MediaImage {
        format: thumbnail.ContentType()?.to_string_lossy().into(),
        data: buf,
      };

      *_thumbnail.write().await = Some(thumbnail);

      Ok(())
    });

    [properties, thumbnail]
  }

  pub async fn update_session(&self, new_session: GlobalSystemMediaTransportControlsSession) {
    {
      *self.session.write().await = new_session;
    }

    self.update().await;
  }

  pub async fn get_elapsed(&self) -> Duration {
    *self.elapsed.read().await
  }

  pub async fn to_metadata(&self) -> MediaMetadata {
    MediaMetadata {
      uid: None,
      uri: None,
      state: *self.state.read().await,
      duration: *self.duration.read().await,
      title: self.title.read().await.clone(),
      album: None,
      artists: vec![self.artist.read().await.clone()],
      cover_url: None,
      cover: self.thumbnail.read().await.clone(),
      background_url: None,
      background: None,
    }
  }
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
