use std::time::Duration;

use tokio::time::sleep;

use currently_playing::platform::MediaListener;

#[tokio::main]
async fn main() -> currently_playing::Result<()> {
  let media = MediaListener::new().await.unwrap();

  loop {
    let metadata = media.poll_async().await?;
    let elapsed = media.poll_elapsed().await?;

    println!("\x1bc");

    println!("Title: {}", metadata.title);
    println!("State: {:?}", metadata.state);
    println!("Length: {:?}", metadata.duration);
    println!("Elapsed: {:?}", elapsed);
    println!("Artist: {:?}", metadata.artists);
    println!("Cover: {:?}", metadata.cover);

    sleep(Duration::from_millis(200)).await;
  }
}
