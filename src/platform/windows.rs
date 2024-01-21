#![cfg(windows)]

use std::sync::{Arc, RwLock};

use windows::core::{Error as WError, Result as WResult, HRESULT};
use windows::Foundation::{EventRegistrationToken, TypedEventHandler};
use windows::Media::Control::{
  CurrentSessionChangedEventArgs, GlobalSystemMediaTransportControlsSession,
  GlobalSystemMediaTransportControlsSessionManager,
  GlobalSystemMediaTransportControlsSessionMediaProperties,
  GlobalSystemMediaTransportControlsSessionPlaybackInfo,
  GlobalSystemMediaTransportControlsSessionPlaybackStatus, MediaPropertiesChangedEventArgs,
  PlaybackInfoChangedEventArgs,
};
use windows::Storage::Streams::{DataReader, IRandomAccessStreamReference};

use crate::{MediaImage, MediaMetadata, MediaState, Result};

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
  media_change_event: Option<(MediaPropertiesChangedEvent, EventRegistrationToken)>,
  playback_info_change_event: Option<(PlaybackInfoChangedEvent, EventRegistrationToken)>,
}

// need to impl these because of MediaPropertiesChangedEvent
unsafe impl Send for SessionManager {}
unsafe impl Sync for SessionManager {}

#[derive(Debug)]
pub struct MediaListener {
  manager: GlobalSystemMediaTransportControlsSessionManager,
  session: Arc<RwLock<WResult<SessionManager>>>,
  handle: tokio::runtime::Handle,
  _event: Option<SessionChangedEvent>,
}

impl MediaListener {
  pub async fn new_with_handle(handle: tokio::runtime::Handle) -> Result<Self> {
    let manager = GlobalSystemMediaTransportControlsSessionManager::RequestAsync()?.await?;

    let this = Self {
      session: Arc::new(RwLock::new(
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

      let Ok(mut session) = handle.write() else {
        return Ok(());
      };

      let Ok(session) = session.as_mut() else {
        return Ok(());
      };

      tokio.block_on(session.update_session(new_session));

      Ok(())
    });

    self.manager.CurrentSessionChanged(&event)?;

    Ok(())
  }

  fn _on_change<F: Fn(MediaMetadata) + Send + Sync + 'static>(
    handle: Arc<RwLock<WResult<SessionManager>>>,
    tokio: tokio::runtime::Handle,
    callback: Arc<F>,
  ) -> WResult<()> {
    let Ok(mut session) = handle.write() else {
      return Ok(());
    };

    let Ok(session) = session.as_mut() else {
      return Ok(());
    };

    let metadata = tokio.block_on(async {
      session.update_all().await;
      session.create_metadata().await
    })?;

    callback(metadata);

    Ok(())
  }

  pub fn on_change<F: Fn(MediaMetadata) + Send + Sync + 'static>(
    &mut self,
    callback: F,
  ) -> Result<()> {
    let self_session = self.session.clone();
    let handle = self.session.clone();
    let tokio = self.handle.clone();

    let callback = Arc::new(callback);

    let media_change_event_callback = callback.clone();

    let media_change_event = MediaPropertiesChangedEvent::new(move |_, _| {
      Self::_on_change(
        handle.clone(),
        tokio.clone(),
        media_change_event_callback.clone(),
      )
    });

    let handle = self.session.clone();
    let tokio = self.handle.clone();
    let playback_info_change_event_callback = callback.clone();

    let playback_info_change_event = PlaybackInfoChangedEvent::new(move |_, _| {
      Self::_on_change(
        handle.clone(),
        tokio.clone(),
        playback_info_change_event_callback.clone(),
      )
    });

    let Ok(mut session) = self_session.write() else {
      return Ok(());
    };

    let Ok(session) = session.as_mut() else {
      return Ok(());
    };

    let media_change_event_token = session
      .session
      .MediaPropertiesChanged(&media_change_event)?;

    let playback_info_change_event_token = session
      .session
      .PlaybackInfoChanged(&playback_info_change_event)?;

    session.media_change_event = Some((media_change_event, media_change_event_token));
    session.playback_info_change_event =
      Some((playback_info_change_event, playback_info_change_event_token));

    Ok(())
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
      media_change_event: None,
      playback_info_change_event: None,
    }
  }

  pub async fn update_session(&mut self, new_session: GlobalSystemMediaTransportControlsSession) {
    if let Some((handler, token)) = self.media_change_event.as_ref() {
      let _ = self.session.RemoveMediaPropertiesChanged(*token);
      let _ = new_session.MediaPropertiesChanged(handler);
    }

    if let Some((handler, token)) = self.playback_info_change_event.as_ref() {
      let _ = self.session.RemoveMediaPropertiesChanged(*token);
      let _ = new_session.PlaybackInfoChanged(handler);
    }

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
      .map(|s| s.to_string_lossy().into())?;

    let album = self
      .properties
      .as_ref()
      .ok()
      .map(GlobalSystemMediaTransportControlsSessionMediaProperties::AlbumTitle)
      .and_then(WResult::ok)
      .map(|s| s.to_string_lossy().into());

    let artist = self
      .properties
      .as_ref()
      .ok()
      .map(GlobalSystemMediaTransportControlsSessionMediaProperties::Artist)
      .and_then(WResult::ok)
      .map(|s| s.to_string_lossy());

    let cover = self.thumbnail.clone().ok();

    let artists = match artist {
      Some(value) => vec![value.into()].into(),
      None => vec![].into(),
    };

    Ok(MediaMetadata {
      uid: None,
      uri: None,
      state: self.playback_status.clone()?.into(),
      duration: Default::default(),
      title,
      album,
      artists,
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

pub async fn get_info() -> WResult<MediaImage> {
  let manager = GlobalSystemMediaTransportControlsSessionManager::RequestAsync()?.await?;
  let session = manager.GetCurrentSession()?;
  let info = session.GetPlaybackInfo()?;
  let status = info.PlaybackStatus()?;
  let props = session.TryGetMediaPropertiesAsync()?.await?;
  let title = props.Title().map(|s| s.to_string_lossy());
  let artist = props.Artist().map(|s| s.to_string_lossy());
  let typ = props.PlaybackType().unwrap().Value();
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

  println!("{props:?}");
  println!("{status:?}");
  println!("{title:?}");
  println!("{artist:?}");
  println!("{typ:?}");
  println!("{thumbnail:?}");

  Ok(image)
}
