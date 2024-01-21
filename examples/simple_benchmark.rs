use std::time::Duration;

use tokio::runtime::Handle;
use tokio::task::spawn_blocking;

use currently_playing::platform::MediaListener;

#[tokio::main]
async fn main() {
  if cfg!(debug_assertions) {
    panic!("Cannot run in debug mode, since it effects performance")
  }

  let handle = Handle::current();

  spawn_blocking(move || {
    let media = handle.block_on(MediaListener::new()).unwrap();
    let duration = Duration::from_secs(5);

    let result = benchmarking::bench_function_with_duration(duration, |m| {
      m.measure(|| handle.block_on(media.poll_async()));
    })
    .unwrap();

    println!(
      "{:?} [{:?}/s] [{:?} in {duration:?}]",
      result.elapsed(),
      result.speed(),
      result.times(),
    );
  });
}
