#![cfg(windows)]

use std::ops::Deref;
use std::sync::Arc;
use std::time::Duration;

use futures_locks::RwLock;
use tokio::runtime::Handle as TokioHandle;
use tokio::task::JoinHandle;
use windows::core::Result as WResult;
use windows::Foundation::TypedEventHandler;
use windows::Media::Control::{
  CurrentSessionChangedEventArgs, GlobalSystemMediaTransportControlsSession,
  GlobalSystemMediaTransportControlsSessionManager,
  GlobalSystemMediaTransportControlsSessionPlaybackStatus, MediaPropertiesChangedEventArgs,
  PlaybackInfoChangedEventArgs, TimelinePropertiesChangedEventArgs,
};
use windows::Storage::Streams::DataReader;

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
  manager: GlobalSystemMediaTransportControlsSessionManager,
  session: Arc<RwLock<WResult<SessionManager>>>,
  handle: TokioHandle,
  _event: Option<SessionChangedEvent>,
}

impl MediaListener {
  pub async fn new_with_handle(handle: TokioHandle) -> Result<Self> {
    let manager = GlobalSystemMediaTransportControlsSessionManager::RequestAsync()?.await?;

    let this = Self {
      session: Arc::new(RwLock::new(
        manager
          .GetCurrentSession()
          .map(|s| SessionManager::new(s, handle.clone())),
      )),
      _event: None,
      handle,
      manager,
    };

    this.setup_events()?;

    Ok(this)
  }

  /// # Panics
  ///
  /// This will panic if called outside the context of a Tokio runtime. ([tokio::runtime::Handle::current])
  ///
  /// use [MediaListener::new_with_handle] instead
  pub async fn new() -> Result<Self> {
    Self::new_with_handle(TokioHandle::current()).await
  }

  pub async fn poll_elapsed(&self, update: bool) -> Result<Duration> {
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

  fn setup_events(&self) -> Result<()> {
    let handle = self.session.clone();
    let tokio = self.handle.clone();

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

    self.manager.CurrentSessionChanged(&event)?;

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
      let title = props.Artist().map(|s| s.to_string_lossy());
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

struct ForceSendSync<T>(pub T);

unsafe impl<T> Send for ForceSendSync<T> {}
unsafe impl<T> Sync for ForceSendSync<T> {}

impl<T> Deref for ForceSendSync<T> {
  type Target = T;

  fn deref(&self) -> &Self::Target {
    &self.0
  }
}
