use futures::prelude::*;

use hyper::{Body, Method, Request, Response};
use kroeg_tap::{Context, EntityStore, QueueStore};
use std::sync::Arc;

use super::ServerError;

/// An alias for the function type of the request handler that is expected to be implemented.
pub type RequestHandler<T, R> = Box<
    Fn(Context, T, R, Request<Body>)
            -> Box<Future<Item = (T, R, Response<Body>), Error = (ServerError<T>, T)> + Send + 'static>
        + Send
        + Sync,
>;

/// A route.
///
/// Kroeg uses its own routing system, to allow easily passing
/// the EntityStore and QueueStore, as well as a small context.
pub struct Route<T: EntityStore, R: QueueStore> {
    pub path: String,
    pub method: Method,
    pub is_prefix: bool,
    pub content_type: Vec<String>,
    pub handler: Arc<RequestHandler<T, R>>,
}

// #[derive(Clone)] does not work here, because it will require all template parameters (T and R)
// to also be Clone. However, cloning the Route will never call clone on the EntityStore or QueueStore.
// So, the only solution is to manually implement Clone.
impl<T: EntityStore, R: QueueStore> Clone for Route<T, R> {
    fn clone(&self) -> Self {
        Route {
            path: self.path.clone(),
            method: self.method.clone(),
            is_prefix: self.is_prefix,
            content_type: self.content_type.clone(),
            handler: self.handler.clone(),
        }
    }
}

impl<T: EntityStore, R: QueueStore> Route<T, R> {
    /// Create a handler for a GET request to a specific path.
    pub fn get(path: &str, handler: RequestHandler<T, R>) -> Self {
        Route {
            path: path.to_owned(),
            method: Method::GET,
            is_prefix: false,
            content_type: Vec::new(),
            handler: Arc::new(handler),
        }
    }
    /// Create a handler for a GET request to a prefix.
    pub fn get_prefix(path: &str, handler: RequestHandler<T, R>) -> Self {
        Route {
            path: path.to_owned(),
            method: Method::GET,
            is_prefix: true,
            content_type: Vec::new(),
            handler: Arc::new(handler),
        }
    }
    /// Create a handler for a GET request to a prefix.
    pub fn post_prefix(path: &str, handler: RequestHandler<T, R>) -> Self {
        Route {
            path: path.to_owned(),
            method: Method::POST,
            is_prefix: true,
            content_type: Vec::new(),
            handler: Arc::new(handler),
        }
    }

    /// Validates if this request can be handled by the route.
    ///
    /// This means: The method has to match, and the path has to either start with or be equal
    /// to the path in the route.
    pub fn can_handle(&self, request: &Request<Body>) -> bool {
        if self.method != *request.method() {
            return false;
        }

        let uri = request.uri();
        if self.is_prefix {
            uri.path().starts_with(&self.path)
        } else {
            uri.path() == &self.path
        }
    }
}
