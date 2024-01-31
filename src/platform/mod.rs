#![allow(unused_imports)]

use std::fmt::{Debug, Formatter};
use std::ops::Deref;
#[cfg(windows)]
pub use windows::*;

#[cfg(target_os = "linux")]
pub use linux::*;

#[cfg(windows)]
pub mod windows;

#[cfg(target_os = "linux")]
pub mod linux;

#[cfg(target_os = "linux")]
pub type SystemMediaSource = MprisMediaSource;

pub(crate) struct ForceSendSync<T>(pub T);

impl<T> Debug for ForceSendSync<T>
  where
    T: Debug,
{
  fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
    T::fmt(&self.0, f)
  }
}

unsafe impl<T> Send for ForceSendSync<T> {}
unsafe impl<T> Sync for ForceSendSync<T> {}

impl<T> Deref for ForceSendSync<T> {
  type Target = T;

  fn deref(&self) -> &Self::Target {
    &self.0
  }
}