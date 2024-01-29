#![cfg(feature = "ws")]

use std::borrow::Cow;
use std::io::ErrorKind;
use std::net::SocketAddr;
use std::sync::{Arc, RwLock};
use std::thread::JoinHandle;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::{Error, Message};
use tokio_tungstenite::{accept_async, WebSocketStream};

use crate::{MediaEvent, MediaMetadata, TokioRuntime};

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
pub struct MediaListener {
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

impl MediaListener {
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
pub struct MediaListenerPooled {
  listener: Arc<MediaListener>,
  runtime: Arc<TokioRuntime>,
  metadata: Arc<RwLock<MediaMetadata>>,
  elapsed: Arc<RwLock<Duration>>,
  background: JoinHandle<()>,
}

impl MediaListenerPooled {
  pub fn new(listener: MediaListener) -> std::io::Result<Self> {
    let listener = Arc::new(listener);
    let runtime = TokioRuntime::new()?;
    let runtime = Arc::new(runtime);
    let metadata = Arc::new(RwLock::new(MediaMetadata::default()));
    let elapsed = Arc::new(RwLock::new(Duration::default()));

    let _listener = listener.clone();
    let _runtime = runtime.clone();
    let _metadata = metadata.clone();
    let _elapsed = elapsed.clone();

    let background = std::thread::spawn(move || {
      _runtime.block_on(background_task(_listener, _metadata, _elapsed));
    });

    Ok(Self {
      listener,
      runtime,
      metadata,
      elapsed,
      background,
    })
  }

  pub fn poll(&self) -> MediaMetadata {
    self.metadata.read().unwrap().clone()
  }

  pub fn poll_elapsed(&self) -> Duration {
    *self.elapsed.read().unwrap()
  }
}

async fn background_task(
  listener: Arc<MediaListener>,
  metadata: Arc<RwLock<MediaMetadata>>,
  elapsed: Arc<RwLock<Duration>>,
) {
  while let Ok(mut connection) = listener.get_connection().await {
    while let Some(Ok(event)) = connection.next().await {
      match event {
        MediaEvent::MediaChanged(info) => {
          *metadata.write().unwrap() = info;
        },
        MediaEvent::StateChanged(state) => {
          metadata.write().unwrap().state = state;
        },
        MediaEvent::ProgressChanged(time) => {
          let duration = metadata.read().unwrap().duration;
          let duration = duration.mul_f64(time);

          *elapsed.write().unwrap() = duration;
        }
      }
    }
  }
}
