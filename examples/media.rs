use tokio::io::{AsyncReadExt, stdin};
use currently_playing::platform::MediaListener;

#[tokio::main]
async fn main() -> currently_playing::Result<()> {
  let mut media = MediaListener::new().await.unwrap();

  media.on_change(|metadata| {
    dbg!(metadata);
  })?;

  stdin().read_exact(&mut [0]).await.unwrap();

  Ok(())
}
