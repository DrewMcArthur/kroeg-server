use dotenv;
use std::{fs::File, io::Read};
use toml;

#[derive(Clone, Debug, Deserialize)]
pub struct Config {
    pub listen: String,
    pub server: ServerConfig,
    pub database: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ServerConfig {
    pub base_uri: String,
    pub instance_id: u32,
    pub admins: Vec<String>,
}

pub fn read_config() -> Config {
    let config_url = dotenv::var("CONFIG").unwrap_or("server.toml".to_owned());
    let mut file = File::open(&config_url).expect("Server config file not found!\nPlease set the config in server.toml or set the CONFIG environment variable!");
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).expect("Failed to read file!");

    toml::from_slice(&buffer).expect("Invalid config file!")
}
