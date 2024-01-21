use std::time::Duration;

use tokio::runtime::Handle;
use tokio::task::spawn_blocking;

use currently_playing::platform::MediaListener;

#[tokio::main]
async fn main() {
  if cfg!(debug_assertions) {
    panic!("Cannot run in debug mode, since it effects performance")
  }

  let media = MediaListener::new().await.unwrap();

  spawn_blocking(move || {
    let duration = Duration::from_secs(5);

    let result = benchmarking::bench_function_with_duration(duration, |m| {
      let handle = Handle::current();

      m.measure(|| handle.block_on(media.poll_async()));
    }).unwrap();

    println!(
      "{:?} [{:?}/s] [{:?} in {duration:?}]",
      result.elapsed(),
      result.speed(),
      result.times(),
    );
  });
}
