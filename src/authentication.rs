//! Simple, insecure, authentication method to mock the server for now.

use super::config;
use hyper::Request;
use kroeg_tap::User;

use std::collections::HashMap;

fn anonymous() -> User {
    User {
        claims: HashMap::new(),
        issuer: None,
        subject: "anonymous".to_owned(),
        audience: vec![],
        token_identifier: "anon".to_owned(),
    }
}

pub fn user_from_request<T>(config: &config::Config, req: &Request<T>) -> User {
    match req
        .headers()
        .get("Authorization")
        .and_then(|f| f.to_str().ok())
    {
        Some(val) => User {
            claims: HashMap::new(),
            issuer: Some(config.server.base_uri.to_owned()),
            subject: val.to_owned(),
            audience: vec![config.server.base_uri.to_owned()],
            token_identifier: "dyngen".to_owned(),
        },

        None => anonymous(),
    }
}
