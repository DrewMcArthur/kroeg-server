//! Simple, insecure, authentication method to mock the server for now.

use super::config;
use hyper::Request;
use kroeg_tap::User;

use std::collections::HashMap;

pub fn user_from_request<T>(config: &config::Config, req: &Request<T>) -> User {
    match req
        .headers()
        .get("Authorization")
        .and_then(|f| f.to_str().ok())
    {
        Some(val) => {
            let mut claims = HashMap::new();
            if config.server.admins.contains(&String::from(val)) {
                claims.insert("admin".to_owned(), "1".to_owned());
            }

            User {
                claims: claims,
                issuer: Some(config.server.base_uri.to_owned()),
                subject: val.to_owned(),
                audience: vec![config.server.base_uri.to_owned()],
                token_identifier: "dyngen".to_owned(),
            }
        }

        None => User {
            claims: HashMap::new(),
            issuer: None,
            subject: "anonymous".to_owned(),
            audience: vec![],
            token_identifier: "anon".to_owned(),
        },
    }
}
