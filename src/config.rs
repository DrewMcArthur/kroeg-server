use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
pub struct ServerConfig {
    pub domain: String,
    pub name: String,
    pub description: String,
    pub instance_id: u32,
    pub admins: Vec<String>,
}
