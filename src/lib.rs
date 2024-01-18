use std::borrow::Cow;
use std::time::Duration;

use serde::{Deserialize, Serialize};

pub mod ws;
pub mod platform;

/// State of what is currently playing
#[derive(Default, Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
pub enum MediaState {
  Playing,
  Paused,
  #[default]
  Stopped,
}

/// Metadata of what is currently playing
#[derive(Default, Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde_with::serde_as]
pub struct MediaMetadata<'a> {
  /// UID of what is currently playing if available
  pub uid: Option<Cow<'a, str>>,
  /// UID of what is currently playing if available
  pub uri: Option<Cow<'a, str>>,
  /// State of what is currently playing
  pub state: MediaState,
  /// Duration of what is currently playing
  #[serde_as(as = "DurationMilliSeconds<u64>")]
  pub duration: Duration,
  /// Title of what is currently playing
  pub title: Cow<'a, str>,
  /// Album of what is currently playing if available
  pub album: Option<Cow<'a, str>>,
  /// Artists of what is currently playing
  pub artists: Cow<'a, [Cow<'a, str>]>,
  /// Cover art of what is currently playing if available
  pub cover_url: Option<Cow<'a, str>>,
  /// Background art of what is currently playing if available
  /// (when you hit the "full screen" thing in the bottom-right corner of spotify)
  pub background_url: Option<Cow<'a, str>>,
}

/// Media Events
#[derive(Debug, Clone, PartialOrd, PartialEq, Serialize, Deserialize)]
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
