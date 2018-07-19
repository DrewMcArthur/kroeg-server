use futures::prelude::*;
use kroeg_tap::{EntityStore, QueueItem, QueueStore};
use std::error::Error;
use std::time::{Duration, Instant};
use tokio::timer::Delay;

#[async]
pub fn deliver_one<T: EntityStore, R: QueueStore>(
    store: T,
    item: R::Item,
) -> Result<(T, R::Item), (T, R::Item, T::Error)> {
    match item.event() {
        "deliver" => {
            println!(" ║ todo: deliver {:?}", item.data());
            Ok((store, item))
        }

        _ => {
            panic!("oh no");
        }
    }
}

#[async]
pub fn loop_deliver<T: EntityStore, R: QueueStore>(mut store: T, queue: R) -> Result<(), ()> {
    println!(" ╔ delivery started\n ╚ ready...");
    loop {
        let item = await!(queue.get_item()).unwrap();
        match item {
            Some(val) => {
                println!(" ╔ got {}", val.event());
                match await!(deliver_one::<T, R>(store, val)) {
                    Ok((s, item)) => {
                        store = s;
                        await!(queue.mark_success(item)).unwrap();
                        println!(" ╚ success");
                    }
                    Err((s, item, e)) => {
                        store = s;
                        await!(queue.mark_failure(item)).unwrap();
                        println!(" ╚ failure");

                        return Err(());
                    }
                };
            }

            None => {
                println!(" ╔ found nothing\n ╚ waiting...");
                await!(Delay::new(Instant::now() + Duration::from_millis(10000))).unwrap();
            }
        };
    }

    Ok(())
}
