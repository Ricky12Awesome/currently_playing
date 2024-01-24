use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::runtime::Handle as TokioHandle;
use tokio::runtime::Runtime as TokioRuntime;

pub type PlatformListener = crate::platform::MediaListener;
pub type WebsocketListener = crate::ws::MediaListener;

use crate::{Error, Result};

#[derive(Debug)]
pub struct MediaListener {
  handle: TokioHandle,
  _runtime: Option<Arc<TokioRuntime>>,
  platform_listener: Option<PlatformListener>,
  websocket_listener: Option<WebsocketListener>,
}

#[derive(
  Default, Debug, Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Hash, Serialize, Deserialize,
)]
pub enum WebsocketSettings {
  Local(u16),
  Addr(SocketAddr),

  #[default]
  Default,
}

impl MediaListener {
  //noinspection DuplicatedCode
  pub fn new(handle: Option<TokioHandle>, ws: WebsocketSettings) -> Result<Self> {
    let handle = handle.or_else(|| TokioHandle::try_current().ok());

    let (handle, runtime) = match handle {
      Some(handle) => (handle, None),
      None => {
        let runtime = TokioRuntime::new().unwrap();

        (runtime.handle().clone(), Some(Arc::new(runtime)))
      }
    };

    let this_handle = Some(handle.clone());
    let this_new = Self::new_async(this_handle, ws);
    let this = handle.block_on(this_new);

    match this {
      Ok(mut this) => {
        this._runtime = runtime;
        Ok(this)
      }
      Err(err) => Err(err),
    }
  }

  //noinspection DuplicatedCode
  pub async fn new_async(handle: Option<TokioHandle>, ws: WebsocketSettings) -> Result<Self> {
    let websocket_listener = match ws {
      WebsocketSettings::Local(port) => WebsocketListener::bind_local(port).await,
      WebsocketSettings::Addr(addr) => WebsocketListener::bind(addr).await,
      WebsocketSettings::Default => WebsocketListener::bind_default().await,
    };

    let platform_listener = PlatformListener::new_async().await;

    // can't use pattern matching cause it moves the value,
    // and std::io::Error doesn't impl Clone
    if websocket_listener.is_err() && platform_listener.is_err() {
      let err1 = Box::new(Error::from(websocket_listener.unwrap_err()));
      let err2 = Box::new(platform_listener.unwrap_err());

      return Err(Error::FailedToCreateListener((err1, err2)));
    }

    let handle = handle.or_else(|| TokioHandle::try_current().ok());

    let (handle, runtime) = match handle {
      Some(handle) => (handle, None),
      None => {
        let runtime = TokioRuntime::new().unwrap();

        (runtime.handle().clone(), Some(Arc::new(runtime)))
      }
    };

    Ok(Self {
      handle,
      _runtime: runtime,
      platform_listener: platform_listener.ok(),
      websocket_listener: websocket_listener.ok(),
    })
  }
}
