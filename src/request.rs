use futures::prelude::*;

use hyper::{
    client::{HttpConnector, ResponseFuture},
    Body, Client, Error, Request, Response,
};
use hyper_tls::HttpsConnector;

use kroeg_tap::{EntityStore, StoreItem};
use std::time::Duration;
use tokio::timer::Timeout;

/// A Future that will follow an amount of requests
/// before retunring the response.
pub struct HyperLDRequest {
    client: Client<HttpsConnector<HttpConnector>>,
    requests_left: u32,
    current_future: Timeout<ResponseFuture>,
}

impl Future for HyperLDRequest {
    type Item = Option<Response<Body>>;
    type Error = Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        loop {
            break match self.current_future.poll() {
                Ok(Async::Ready(response)) => {
                    if let Some(location) = response
                        .headers()
                        .get("Location")
                        .and_then(|f| f.to_str().ok())
                    {
                        if self.requests_left == 0 {
                            break Ok(Async::Ready(None));
                        }

                        let request = Request::get(location.to_owned())
                         .header("Accept", "application/ld+json; profile=\"https://www.w3.org/ns/activitystreams\", application/activity+json, application/json")
                         .body(Body::default())
                         .unwrap();

                        self.requests_left -= 1;
                        self.current_future =
                            Timeout::new(self.client.request(request), Duration::from_millis(7000));
                        continue;
                    }

                    Ok(Async::Ready(Some(response)))
                }

                Ok(Async::NotReady) => Ok(Async::NotReady),
                Err(e) => {
                    if e.is_elapsed() {
                        Ok(Async::Ready(None))
                    } else {
                        Err(e.into_inner().unwrap())
                    }
                }
            };
        }
    }
}

impl HyperLDRequest {
    pub fn new(url: &str) -> Self {
        let request = Request::get(url.to_owned())
            .header("Accept", "application/ld+json; profile=\"https://www.w3.org/ns/activitystreams\", application/activity+json, application/json")
            .body(Body::default())
            .unwrap();

        let connector = HttpsConnector::new(1).unwrap();
        let client = Client::builder().build(connector);
        let future = Timeout::new(client.request(request), Duration::from_millis(7000));

        HyperLDRequest {
            client: client,
            requests_left: 2,
            current_future: future,
        }
    }
}

enum StoreState<T: EntityStore + 'static> {
    GetOriginal(T::GetFuture),
    WriteNew(T::StoreFuture),
    Idle(T),
}

pub struct StoreAllFuture<T: EntityStore + 'static> {
    todo: Vec<StoreItem>,
    state: Option<StoreState<T>>,
}

impl<T: EntityStore + 'static> StoreAllFuture<T> {
    pub fn new(store: T, items: Vec<StoreItem>) -> Self {
        StoreAllFuture {
            todo: items,
            state: Some(StoreState::Idle(store)),
        }
    }
}

impl<T: EntityStore + 'static> Future for StoreAllFuture<T> {
    type Item = T;
    type Error = (T::Error, T);

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        loop {
            let new_state = match self.state.take() {
                Some(StoreState::Idle(store)) => {
                    if self.todo.len() > 0 {
                        StoreState::GetOriginal(store.get(self.todo[0].id().to_owned(), true))
                    } else {
                        break Ok(Async::Ready(store));
                    }
                }

                Some(StoreState::GetOriginal(ref mut future)) => match future.poll() {
                    Ok(Async::Ready((Some(mut prev_item), store))) => {
                        let mut item = self.todo.remove(0);
                        if prev_item.meta()[kroeg!(instance)] == item.meta()[kroeg!(instance)] {
                            StoreState::WriteNew(store.put(item.id().to_owned(), item))
                        } else {
                            StoreState::Idle(store)
                        }
                    }

                    Ok(Async::Ready((None, store))) => {
                        let item = self.todo.remove(0);
                        StoreState::WriteNew(store.put(item.id().to_owned(), item))
                    }

                    Ok(Async::NotReady) => break Ok(Async::NotReady),
                    Err(e) => break Err(e),
                },

                Some(StoreState::WriteNew(ref mut future)) => match future.poll() {
                    Ok(Async::Ready((_, store))) => StoreState::Idle(store),

                    Ok(Async::NotReady) => break Ok(Async::NotReady),
                    Err(e) => break Err(e),
                },

                None => break Ok(Async::NotReady),
            };

            self.state = Some(new_state);
        }
    }
}
