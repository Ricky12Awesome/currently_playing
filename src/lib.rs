use std::fmt::{Debug, Display, Formatter};
use std::hash::Hash;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio_tungstenite::tungstenite;

pub mod listener;
pub mod platform;
pub mod ws;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Error)]
#[error(transparent)]
pub enum Error {
  #[cfg(windows)]
  Platform(windows::core::Error),

  #[cfg(target_os = "linux")]
  Platform(#[from] platform::linux::MprisError),

  #[error("No media found or is currently opened")]
  NotExist,

  #[error("Not enabled")]
  NotEnabled,

  #[error("Closed")]
  Closed,

  Timeout(#[from] std::sync::mpsc::RecvTimeoutError),

  Io(#[from] std::io::Error),

  Tungstenite(#[from] tungstenite::Error),

  Other(#[from] anyhow::Error),

  #[error("{}, {}", 0.0, 0.1)]
  FailedToCreateListener((Box<Self>, Box<Self>)),
}
#[cfg(windows)]
#[allow(overflowing_literals)]
impl From<windows::core::Error> for Error {
  fn from(value: windows::core::Error) -> Self {
    use windows::core::HRESULT;

    match value.code() {
      HRESULT(0x00000000) => Self::NotExist,
      HRESULT(0x800706BA) => Self::NotExist,
      _ => Self::Platform(value),
    }
  }
}

/// State of what is currently playing
#[derive(
  Default, Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize,
)]
pub enum MediaState {
  Playing,
  Paused,
  #[default]
  Stopped,
}

/// Image Format
#[derive(Debug, Clone, Ord, PartialOrd, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum ImageFormat {
  #[serde(alias = "image/png")]
  PNG,
  #[serde(alias = "image/jpeg")]
  #[serde(alias = "image/jpg")]
  JPEG,
  #[serde(alias = "image/webp")]
  WEBP,
  #[serde(untagged)]
  Other(String),
}

/// Media Image data
#[derive(Clone, Ord, PartialOrd, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct MediaImage {
  pub format: ImageFormat,
  pub data: Vec<u8>,
}

impl Debug for MediaImage {
  fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("MediaImage")
      .field("format", &self.format)
      .field("data", &format!("vec![u8; {}]", self.data.len()))
      .finish()
  }
}

impl Display for ImageFormat {
  fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
    match self {
      Self::PNG => write!(f, "image/png"),
      Self::JPEG => write!(f, "image/jpeg"),
      Self::WEBP => write!(f, "image/webp"),
      Self::Other(format) => write!(f, "{}", format),
    }
  }
}

impl From<String> for ImageFormat {
  fn from(value: String) -> Self {
    match value.as_str() {
      "image/png" => Self::PNG,
      "image/jpg" | "image/jpeg" => Self::JPEG,
      "image/webp" => Self::WEBP,
      _ => Self::Other(value),
    }
  }
}

/// Metadata of what is currently playing
#[serde_with::serde_as]
#[derive(Default, Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub struct MediaMetadata {
  /// UID of what is currently playing if available
  pub uid: Option<String>,
  /// UID of what is currently playing if available
  pub uri: Option<String>,
  /// State of what is currently playing
  pub state: MediaState,
  /// Duration of what is currently playing
  #[serde_as(as = "::serde_with::DurationMilliSeconds<u64>")]
  pub duration: Duration,
  #[serde_as(as = "::serde_with::DurationMilliSeconds<u64>")]
  /// Elapsed duration of what is currently playing
  pub elapsed: Duration,
  /// Title of what is currently playing
  pub title: String,
  /// Album of what is currently playing if available
  pub album: Option<String>,
  /// Artists of what is currently playing
  pub artists: Vec<String>,
  /// Cover art url of what is currently playing if available
  pub cover_url: Option<String>,
  /// Cover art image data of what is currently playing if available
  pub cover: Option<MediaImage>,
  /// Background art url of what is currently playing if available
  /// (when you hit the "full screen" thing in the bottom-right corner of spotify)
  pub background_url: Option<String>,
  /// Background art image data of what is currently playing if available
  /// (when you hit the "full screen" thing in the bottom-right corner of spotify)
  pub background: Option<MediaImage>,
}

impl MediaMetadata {
  pub fn merge(self, fallback: MediaMetadata) -> MediaMetadata {
    MediaMetadata {
      uid: self.uid.or(fallback.uid),
      uri: self.uri.or(fallback.uri),
      state: self.state,
      duration: if self.duration == Duration::default() {
        fallback.duration
      } else {
        self.duration
      },
      elapsed: if self.elapsed == Duration::default() {
        fallback.elapsed
      } else {
        self.elapsed
      },
      title: if self.title.is_empty() {
        fallback.title
      } else {
        self.title
      },
      album: self.album.or(fallback.album),
      artists: if self.artists.is_empty() {
        fallback.artists
      } else {
        self.artists
      },
      cover_url: self.cover_url.or(fallback.cover_url),
      cover: self.cover.or(fallback.cover),
      background_url: self.background_url.or(fallback.background_url),
      background: self.background.or(fallback.background),
    }
  }

  pub fn is_different(&self, other: &Self) -> bool {
    let uid = self.uid.is_some() && self.uid != other.uid;
    let uri = self.uri.is_some() && self.uri != other.uri;
    let title = self.title != other.title;
    let artists = self.artists != other.artists;

    uid || uri || (title && artists)
  }
}

/// Media Events
#[serde_with::serde_as]
#[derive(Debug, Clone, PartialOrd, PartialEq, Serialize, Deserialize)]
#[allow(clippy::large_enum_variant)]
pub enum MediaEvent {
  /// Event for when media changed (like going to next song)
  MediaChanged(MediaMetadata),
  /// Event for when state is changed (like when pausing song)
  StateChanged(MediaState),
  /// Event for when progress is updated, usually called on a set interval
  ///
  /// value is a percentage of the duration
  ProgressChanged(#[serde_as(as = "::serde_with::DurationMilliSeconds<u64>")] Duration),
}
