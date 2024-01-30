// use std::time::Duration;
// use tokio::time::sleep;
//
// use currently_playing::platform::MprisMediaSource;
//
// #[tokio::main]
// async fn main() -> currently_playing::Result<()> {
//   let media = MprisMediaSource::new_async().await?;
//
//   loop {
//     let metadata = media.poll_async(true).await?;
//     let elapsed = media.poll_elapsed_async(true).await?;
//
//     println!("\x1bc");
//
//     println!("Title: {}", metadata.title);
//     println!("State: {:?}", metadata.state);
//     println!("Length: {:?}", metadata.duration);
//     println!("Elapsed: {:?}", elapsed);
//     println!("Artist: {:?}", metadata.artists);
//     println!("Cover: {:?}", metadata.cover);
//     println!("Cover (URL): {:?}", metadata.cover_url);
//
//     sleep(Duration::from_millis(500)).await;
//   }
// }
