use http::{request::Parts, Method};
use http_service::{Request, Response};
use kroeg_tap::Context;

use crate::ServerError;

/// An alias for the function type of the request handler that is expected to be implemented.

#[async_trait::async_trait]
pub trait RequestHandler: Send + Sync + 'static {
    async fn run(
        &self,
        context: &mut Context<'_, '_>,
        request: Request,
    ) -> Result<Response, ServerError>;
}

/// A route.
///
/// Kroeg uses its own routing system, to allow easily passing
/// the EntityStore and QueueStore, as well as a small context.
pub struct Route {
    pub path: String,
    pub method: Method,
    pub is_prefix: bool,
    pub content_type: Vec<String>,
    pub handler: Box<dyn RequestHandler>,
}

impl Route {
    /// Create a handler for a GET request to a specific path.
    pub fn get(path: &str, handler: impl RequestHandler) -> Self {
        Route {
            path: path.to_owned(),
            method: Method::GET,
            is_prefix: false,
            content_type: Vec::new(),
            handler: Box::new(handler),
        }
    }
    /// Create a handler for a POST request to a specific path.
    pub fn post(path: &str, handler: impl RequestHandler) -> Self {
        Route {
            path: path.to_owned(),
            method: Method::POST,
            is_prefix: false,
            content_type: Vec::new(),
            handler: Box::new(handler),
        }
    }
    /// Create a handler for a GET request to a prefix.
    pub fn get_prefix(path: &str, handler: impl RequestHandler) -> Self {
        Route {
            path: path.to_owned(),
            method: Method::GET,
            is_prefix: true,
            content_type: Vec::new(),
            handler: Box::new(handler),
        }
    }
    /// Create a handler for a GET request to a prefix.
    pub fn post_prefix(path: &str, handler: impl RequestHandler) -> Self {
        Route {
            path: path.to_owned(),
            method: Method::POST,
            is_prefix: true,
            content_type: Vec::new(),
            handler: Box::new(handler),
        }
    }

    /// Validates if this request can be handled by the route.
    ///
    /// This means: The method has to match, and the path has to either start with or be equal
    /// to the path in the route.
    pub fn can_handle(&self, request: &Parts) -> bool {
        if self.method != request.method {
            return false;
        }

        let uri = &request.uri;
        if self.is_prefix {
            uri.path().starts_with(&self.path)
        } else {
            uri.path() == &self.path
        }
    }
}
