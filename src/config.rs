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
}
