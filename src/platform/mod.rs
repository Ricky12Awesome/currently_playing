#![allow(unused_imports)]

use std::fmt::{Debug, Formatter};
use std::ops::Deref;
use std::sync::RwLockReadGuard;
#[cfg(windows)]
pub use windows::*;

#[cfg(target_os = "linux")]
pub use linux::*;
use crate::listener::{MediaSource, MediaSourceConfig};
use crate::{MediaEvent, MediaMetadata};

#[cfg(windows)]
pub mod windows;

#[cfg(target_os = "linux")]
pub mod linux;

#[cfg(target_os = "linux")]
pub type SystemMediaSource = MprisMediaSource;

#[cfg(windows)]
pub type SystemMediaSource = WindowsMediaSource;