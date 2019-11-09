use http::{request::Parts, Method, StatusCode};
use http_service::{Body, Request, Response};
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

        if !self.content_type.is_empty() {
            let mut matches_content = false;
            let items = request
                .headers
                .get("Accept")
                .map(|v| v.to_str().unwrap_or(""))
                .unwrap_or("")
                .split(',');
            for item in items {
                if self.content_type.iter().any(|f| f == item) {
                    matches_content = true;
                }
            }

            if !matches_content {
                return false;
            }
        }

        let uri = &request.uri;
        if self.is_prefix {
            uri.path().starts_with(&self.path)
        } else {
            uri.path() == &self.path
        }
    }
}

/// Helper function, that allows the router to fall back to 404 easily.
pub async fn not_found(
    _context: &mut Context<'_, '_>,
    _request: Request,
) -> Result<Response, ServerError> {
    Ok(http::Response::builder()
        .status(StatusCode::NOT_FOUND)
        .body(Body::from("OwO I don't know what this is"))
        .unwrap())
}
