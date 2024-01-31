#![cfg(feature = "ws")]

use std::borrow::Cow;
use std::io::ErrorKind;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, RwLock, RwLockReadGuard};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender};
use std::thread::JoinHandle;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::Builder;
use tokio_tungstenite::{accept_async, WebSocketStream};
use tokio_tungstenite::tungstenite::{Error, Message};

use crate::{MediaEvent, MediaMetadata};
use crate::listener::{MediaSource, MediaSourceConfig, WebsocketAddr};

/// Wraps around [TcpListener]
///
/// Examples
/// --------
///
/// ```rs
/// use currently_playing::MediaEvent;
/// use currently_playing::ws::MediaListener;
///
/// // Create a listener using local ip and default port
/// let listener = MediaListener::bind_default().await.unwrap();
///
/// // Create a listener using local ip with custom port
/// let listener = MediaListener::bind_local(69420).await.unwrap();
///
/// // Create a listener using a custom address
/// let listener = MediaListener::bind("127.0.0.1:69420".into()).await.unwrap();
///
/// // Listen for incoming connections, if media client closes, the loop keeps listening
/// while let Ok(mut connection) = listener.get_connection().await {
///   // handle connection
/// }
/// ```
#[derive(Debug)]
pub struct WebsocketMediaSource {
  pub listener: TcpListener,
}

/// Message to send to media client
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MediaMessage {
  /// Updates the progress update interval from the media client
  ProgressUpdateInterval(u64),
}

#[derive(Debug)]
pub struct MediaConnection {
  pub ws: WebSocketStream<TcpStream>,
}

impl MediaConnection {
  fn handle_message(message: Cow<str>) -> Result<MediaEvent, Error> {
    serde_json::from_str::<MediaEvent>(&message)
      .map_err(|err| Error::Io(std::io::Error::new(ErrorKind::InvalidData, err)))
  }

  /// Sets how often it should update the progress
  ///
  /// **This might be ignored depending on the media client implementation**
  pub async fn set_progress_interval(&mut self, interval: Duration) -> Result<(), Error> {
    let ms = interval.as_millis() as u64;
    let interval = MediaMessage::ProgressUpdateInterval(ms);
    let text = serde_json::to_string(&interval).unwrap_or_else(|_| {
      // only panics if serialize was implemented incorrectly
      panic!(
        "failed to turn {} into a json string",
        std::any::type_name::<MediaMessage>()
      )
    });

    self.ws.send(Message::Text(text)).await
  }

  pub async fn close(&mut self) -> Result<(), Error> {
    self.ws.close(None).await
  }

  /// Waits for the next message to be received
  pub async fn next(&mut self) -> Option<Result<MediaEvent, Error>> {
    let message = self.ws.next().await?;

    match message {
      Ok(Message::Text(message)) => {
        let event = Self::handle_message(message.into());

        Some(event)
      }
      Ok(_) => Some(Err(Error::Io(std::io::Error::new(
        ErrorKind::Unsupported,
        "Unsupported message type, only supports Text",
      )))),
      Err(err) => Some(Err(err)),
    }
  }
}

impl WebsocketMediaSource {
  /// Binds to 127.0.0.1:19532
  pub async fn bind_default() -> std::io::Result<Self> {
    Self::bind_local(19532).await
  }

  /// Binds to 127.0.0.1 with a custom port
  pub async fn bind_local(port: u16) -> std::io::Result<Self> {
    let ip = format!("127.0.0.1:{port}");

    Self::bind(ip.parse().unwrap()).await
  }

  /// Binds to the given address, same as calling [TcpListener::bind(addr)]
  pub async fn bind(addr: SocketAddr) -> std::io::Result<Self> {
    let listener = TcpListener::bind(addr).await?;

    Ok(Self { listener })
  }

  /// Binds from [WebsocketAddr]
  pub async fn bind_from(value: WebsocketAddr) -> std::io::Result<Self> {
    match value {
      WebsocketAddr::Local(port) => Self::bind_local(port).await,
      WebsocketAddr::Addr(addr) => Self::bind(addr).await,
      WebsocketAddr::Default => Self::bind_default().await,
    }
  }

  /// Establishes a websocket connection to the client
  pub async fn get_connection(&self) -> Result<MediaConnection, Error> {
    let listener = self.listener.accept().await;
    let (stream, _) = listener.map_err(|_| Error::ConnectionClosed)?;
    let ws = accept_async(stream).await?;

    Ok(MediaConnection { ws })
  }
}

#[derive(Debug)]
#[allow(unused)]
pub struct WebsocketMediaSourceBackground {
  cancel_token: Arc<AtomicBool>,
  metadata: Arc<RwLock<MediaMetadata>>,
  recv: Arc<Mutex<Receiver<MediaEvent>>>,
  _background_task: JoinHandle<()>,
}

impl MediaSource for WebsocketMediaSourceBackground {
  fn create(cfg: MediaSourceConfig) -> crate::Result<Self> {
    if !cfg.websocket_enabled {
      return Err(crate::Error::NotEnabled);
    }

    let cancel_token = Arc::new(AtomicBool::new(false));
    let metadata = Arc::new(RwLock::new(MediaMetadata::default()));
    let (send, recv) = std::sync::mpsc::sync_channel(0);

    let background_task = spawn_background_task(
      cfg.addr,
      cancel_token.clone(),
      metadata.clone(),
      send.clone(),
    );

    let recv = Arc::new(Mutex::new(recv));

    Ok(Self {
      cancel_token,
      metadata,
      recv,
      _background_task: background_task,
    })
  }

  fn is_closed(&self) -> bool {
    self.cancel_token.load(Ordering::SeqCst)
  }

  fn poll(&self) -> crate::Result<MediaMetadata> {
    self.poll_guarded().map(|v| v.clone())
  }

  fn poll_guarded(&self) -> crate::Result<RwLockReadGuard<MediaMetadata>> {
    if self.is_closed() {
      return Err(crate::Error::Closed);
    }

    Ok(self.metadata.read().unwrap())
  }

  fn next(&self) -> crate::Result<MediaEvent> {
    if self.is_closed() {
      return Err(crate::Error::Closed);
    }

    let timeout = Duration::from_millis(1000);
    let recv = self.recv.lock().unwrap();
    let event = recv.recv_timeout(timeout)?;

    Ok(event)
  }
}

fn spawn_background_task(
  addr: WebsocketAddr,
  cancel_token: Arc<AtomicBool>,
  metadata: Arc<RwLock<MediaMetadata>>,
  send: SyncSender<MediaEvent>,
) -> JoinHandle<()> {
  std::thread::spawn(move || {
    let runtime = Builder::new_multi_thread()
      .worker_threads(4)
      .enable_all()
      .build()
      .unwrap();

    loop {
      if cancel_token.load(Ordering::SeqCst) {
        return;
      };

      let source = WebsocketMediaSource::bind_from(addr);
      let result = runtime.block_on(source);

      match result {
        Ok(source) => {
          let task = background_task(source, cancel_token.clone(), metadata.clone(), send.clone());

          runtime.block_on(task);
        }
        Err(_) => std::thread::sleep(Duration::from_millis(1000)),
      }
    }
  })
}

async fn background_task(
  source: WebsocketMediaSource,
  cancel_token: Arc<AtomicBool>,
  metadata: Arc<RwLock<MediaMetadata>>,
  send: SyncSender<MediaEvent>,
) {
  while let Ok(mut connection) = source.get_connection().await {
    if cancel_token.load(Ordering::SeqCst) {
      let _ = connection.close().await;
      return;
    };

    while let Some(Ok(event)) = connection.next().await {
      if cancel_token.load(Ordering::SeqCst) {
        let _ = connection.close().await;
        return;
      };

      let _ = send.try_send(event.clone());

      match event {
        MediaEvent::MediaChanged(info) => {
          *metadata.write().unwrap() = info;
        }
        MediaEvent::StateChanged(state) => {
          metadata.write().unwrap().state = state;
        }
        MediaEvent::ProgressChanged(new_elapsed) => {
          metadata.write().unwrap().elapsed = new_elapsed;
        }
      }
    }
  }
}
