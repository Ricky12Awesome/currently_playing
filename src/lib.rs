use std::any::type_name;
use std::borrow::Cow;
use std::fmt::{Debug, Display, Formatter};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub mod platform;
pub mod ws;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Error)]
#[error(transparent)]
pub enum Error {
  #[cfg(windows)]
  Platform(#[from] windows::core::Error),
  Other(#[from] anyhow::Error),
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
#[derive(Debug, Clone, Ord, PartialOrd, Eq, PartialEq, Serialize, Deserialize)]
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
#[derive(Clone, Ord, PartialOrd, Eq, PartialEq, Serialize, Deserialize)]
pub struct MediaImage {
  pub format: ImageFormat,
  pub data: Vec<u8>,
}

impl Debug for MediaImage {
  fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
    f.debug_struct(type_name::<Self>())
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
pub struct MediaMetadata<'a> {
  /// UID of what is currently playing if available
  pub uid: Option<Cow<'a, str>>,
  /// UID of what is currently playing if available
  pub uri: Option<Cow<'a, str>>,
  /// State of what is currently playing
  pub state: MediaState,
  /// Duration of what is currently playing
  #[serde_as(as = "::serde_with::DurationMilliSeconds<u64>")]
  pub duration: Duration,
  /// Title of what is currently playing
  pub title: Cow<'a, str>,
  /// Album of what is currently playing if available
  pub album: Option<Cow<'a, str>>,
  /// Artists of what is currently playing
  pub artists: Cow<'a, [Cow<'a, str>]>,
  /// Cover art url of what is currently playing if available
  pub cover_url: Option<Cow<'a, str>>,
  /// Cover art image data of what is currently playing if available
  pub cover: Option<MediaImage>,
  /// Background art url of what is currently playing if available
  /// (when you hit the "full screen" thing in the bottom-right corner of spotify)
  pub background_url: Option<Cow<'a, str>>,
  /// Background art image data of what is currently playing if available
  /// (when you hit the "full screen" thing in the bottom-right corner of spotify)
  pub background: Option<MediaImage>,
}

/// Media Events
#[derive(Debug, Clone, PartialOrd, PartialEq, Serialize, Deserialize)]
#[allow(clippy::large_enum_variant)]
pub enum MediaEvent<'a> {
  /// Event for when media changed (like going to next song)
  MediaChanged(MediaMetadata<'a>),
  /// Event for when state is changed (like when pausing song)
  StateChanged(MediaState),
  /// Event for when progress is updated, usually called on a set interval
  ///
  /// value is a percentage of the duration
  ProgressChanged(f64),
}
