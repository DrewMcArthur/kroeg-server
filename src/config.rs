use serde::Deserialize;
use std::{fs::File, io::Read};
use toml;

#[derive(Clone, Debug, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub database: DatabaseConfig,

    pub listen: Option<String>,
    pub deliver: Option<u32>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct DatabaseConfig {
    pub server: String,
    pub username: String,
    pub password: String,
    pub database: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ServerConfig {
    pub base_uri: String,
    pub name: String,
    pub description: String,
    pub instance_id: u32,
    pub admins: Vec<String>,
}

pub fn read_config(path: &str) -> Config {
    let mut file = File::open(path).expect("Server config file not found!");
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).expect("Failed to read file!");

    toml::from_slice(&buffer).expect("Invalid config file!")
}
