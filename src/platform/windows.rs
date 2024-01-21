#![cfg(windows)]

use std::sync::Arc;

use futures_util::lock::Mutex;
use windows::core::{Error as WError, HRESULT, Result as WResult};
use windows::Foundation::TypedEventHandler;
use windows::Media::Control::{CurrentSessionChangedEventArgs, GlobalSystemMediaTransportControlsSession, GlobalSystemMediaTransportControlsSessionManager, GlobalSystemMediaTransportControlsSessionMediaProperties, GlobalSystemMediaTransportControlsSessionPlaybackInfo, GlobalSystemMediaTransportControlsSessionPlaybackStatus, MediaPropertiesChangedEventArgs, PlaybackInfoChangedEventArgs, TimelinePropertiesChangedEventArgs};
use windows::Storage::Streams::{DataReader, IRandomAccessStreamReference};

use crate::{MediaImage, MediaMetadata, MediaState, Result};

pub type TimelinePropertiesChangedEvent = TypedEventHandler<GlobalSystemMediaTransportControlsSession, TimelinePropertiesChangedEventArgs>;
pub type PlaybackInfoChangedEvent =
  TypedEventHandler<GlobalSystemMediaTransportControlsSession, PlaybackInfoChangedEventArgs>;

pub type MediaPropertiesChangedEvent =
  TypedEventHandler<GlobalSystemMediaTransportControlsSession, MediaPropertiesChangedEventArgs>;

pub type SessionChangedEvent = TypedEventHandler<
  GlobalSystemMediaTransportControlsSessionManager,
  CurrentSessionChangedEventArgs,
>;

#[derive(Debug)]
struct SessionManager {
  session: GlobalSystemMediaTransportControlsSession,
  playback_info: WResult<GlobalSystemMediaTransportControlsSessionPlaybackInfo>,
  playback_status: WResult<GlobalSystemMediaTransportControlsSessionPlaybackStatus>,
  properties: WResult<GlobalSystemMediaTransportControlsSessionMediaProperties>,
  thumbnail: WResult<MediaImage>,
}

// need to impl these because of MediaPropertiesChangedEvent
unsafe impl Send for SessionManager {}
unsafe impl Sync for SessionManager {}

#[derive(Debug)]
pub struct MediaListener {
  manager: GlobalSystemMediaTransportControlsSessionManager,
  session: Arc<Mutex<WResult<SessionManager>>>,
  handle: tokio::runtime::Handle,
  _event: Option<SessionChangedEvent>,
}

unsafe impl Send for MediaListener {}
unsafe impl Sync for MediaListener {}

impl MediaListener {
  pub async fn new_with_handle(handle: tokio::runtime::Handle) -> Result<Self> {
    let manager = GlobalSystemMediaTransportControlsSessionManager::RequestAsync()?.await?;

    let this = Self {
      session: Arc::new(Mutex::new(
        manager.GetCurrentSession().map(SessionManager::new),
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
    Self::new_with_handle(tokio::runtime::Handle::current()).await
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
        let mut session = handle.lock().await;

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

  pub async fn poll_async(&self) -> Result<MediaMetadata> {
    let mut session = self.session.lock().await;
    let session = session.as_mut().map_err(|err| err.clone())?;

    session.update_all().await;
    let metadata = session.create_metadata().await?;

    Ok(metadata)
  }
}

impl SessionManager {
  fn new(session: GlobalSystemMediaTransportControlsSession) -> Self {
    Self {
      session,
      playback_info: Err(WError::new(HRESULT(0), "uninitialized".into())),
      playback_status: Err(WError::new(HRESULT(0), "uninitialized".into())),
      properties: Err(WError::new(HRESULT(0), "uninitialized".into())),
      thumbnail: Err(WError::new(HRESULT(0), "uninitialized".into())),
    }
  }

  pub async fn update_session(&mut self, new_session: GlobalSystemMediaTransportControlsSession) {
    self.session = new_session;
    self.update_all().await;
  }

  pub fn update_playback_info(&mut self) {
    self.playback_info = self.session.GetPlaybackInfo();

    self.playback_status = self
      .playback_info
      .as_ref()
      .map_err(WError::clone)
      .and_then(GlobalSystemMediaTransportControlsSessionPlaybackInfo::PlaybackStatus);
  }

  pub async fn update_properties(&mut self) {
    let properties = self.session.TryGetMediaPropertiesAsync();

    self.properties = match properties {
      Ok(value) => match value.await {
        Ok(value) => Ok(value),
        Err(err) => Err(err),
      },
      Err(err) => Err(err),
    };
  }

  pub async fn update_thumbnail(&mut self) {
    let thumbnail = self
      .properties
      .as_ref()
      .map_err(WError::clone)
      .and_then(GlobalSystemMediaTransportControlsSessionMediaProperties::Thumbnail);

    self.thumbnail = match thumbnail {
      Ok(value) => match Self::_update_thumbnail(value).await {
        Ok(value) => Ok(value),
        Err(err) => Err(err),
      },
      Err(err) => Err(err),
    };
  }

  async fn _update_thumbnail(thumbnail: IRandomAccessStreamReference) -> WResult<MediaImage> {
    let thumbnail = thumbnail.OpenReadAsync()?.await?;

    let size = thumbnail.Size()?;
    let pos = thumbnail.Position()?;
    let stream = thumbnail.GetInputStreamAt(pos)?;
    let reader = DataReader::CreateDataReader(&stream)?;

    let mut buf = vec![0u8; size as _];

    reader.LoadAsync(size as _)?.await?;
    reader.ReadBytes(&mut buf)?;

    thumbnail.Close()?;

    let image = MediaImage {
      format: thumbnail.ContentType()?.to_string_lossy().into(),
      data: buf,
    };

    Ok(image)
  }

  pub async fn update_all(&mut self) {
    self.update_playback_info();
    self.update_properties().await;
    self.update_thumbnail().await;
  }

  pub async fn create_metadata(&self) -> WResult<MediaMetadata> {
    let title = self
      .properties
      .as_ref()
      .map_err(WError::clone)
      .and_then(GlobalSystemMediaTransportControlsSessionMediaProperties::Title)
      .map(|s| s.to_string_lossy())?;

    let album = self
      .properties
      .as_ref()
      .ok()
      .map(GlobalSystemMediaTransportControlsSessionMediaProperties::AlbumTitle)
      .and_then(WResult::ok)
      .map(|s| s.to_string_lossy());

    let artist = self
      .properties
      .as_ref()
      .ok()
      .map(GlobalSystemMediaTransportControlsSessionMediaProperties::Artist)
      .and_then(WResult::ok)
      .map(|s| s.to_string_lossy());

    let cover = self.thumbnail.clone().ok();

    Ok(MediaMetadata {
      uid: None,
      uri: None,
      state: self.playback_status.clone()?.into(),
      duration: Default::default(),
      title,
      album,
      artists: artist.into_iter().collect(),
      cover_url: None,
      cover,
      background_url: None,
      background: None,
    })
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

pub async fn get_info() -> WResult<()> {
  let manager = GlobalSystemMediaTransportControlsSessionManager::RequestAsync()?.await?;
  let session = manager.GetCurrentSession()?;
  let props = session.TryGetMediaPropertiesAsync()?.await?;
  let thumbnail = props.Thumbnail()?.OpenReadAsync()?.await?;

  let size = thumbnail.Size()?;
  let pos = thumbnail.Position()?;
  let stream = thumbnail.GetInputStreamAt(pos)?;
  let reader = DataReader::CreateDataReader(&stream)?;

  let mut buf = vec![0u8; size as _];

  reader.LoadAsync(size as _)?.await?;
  reader.ReadBytes(&mut buf)?;

  thumbnail.Close()?;

  let image = MediaImage {
    format: thumbnail.ContentType()?.to_string_lossy().into(),
    data: buf,
  };

  dbg!(image);
  let event1 = TimelinePropertiesChangedEvent::new(|sender, value| {
    println!("Timeline: {sender:?} {value:?}");
    Ok(())
  });

  let event2 = MediaPropertiesChangedEvent::new(|sender, value| {
    println!("Media: {sender:?} {value:?}");
    Ok(())
  });

  let event3 = PlaybackInfoChangedEvent::new(|sender, value| {
    println!("Playback: {sender:?} {value:?}");
    Ok(())
  });

  let event4 = SessionChangedEvent::new(|sender, value| {
    println!("Session: {sender:?} {value:?}");
    Ok(())
  });

  let _t1 = session.TimelinePropertiesChanged(&event1)?;
  let _t2 = session.MediaPropertiesChanged(&event2)?;
  let _t3 = session.PlaybackInfoChanged(&event3)?;
  let _t4 = manager.CurrentSessionChanged(&event4);

  Ok(())
}
