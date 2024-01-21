use tokio::io::AsyncReadExt;
use currently_playing::platform::get_info;

#[tokio::main]
async fn main() {
  get_info().await.unwrap();
  tokio::io::stdin().read_exact(&mut [0]).await.unwrap();
}