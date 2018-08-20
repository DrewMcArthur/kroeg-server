use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetadataItem {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    pub username: String,
    pub acct: String,
    pub display_name: String,
    pub locked: bool,
    pub created_at: String,
    pub followers_count: u32,
    pub following_count: u32,
    pub statuses_count: u32,
    pub note: String,
    pub url: String,
    pub avatar: String,
    pub avatar_static: String,
    pub header: String,
    pub header_static: String,
    pub moved: Option<String>, // ??
    pub fields: Vec<MetadataItem>,
    pub bot: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Application {
    pub name: String,
    pub website: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    pub id: String,
    #[serde(rename = "type")]
    pub typeval: String,
    pub url: String,
    pub remote_url: Option<String>,
    pub preview_url: String,
    pub text_url: String,
    pub meta: HashMap<String, String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Card {
    pub url: String,
    pub title: String,
    pub description: String,
    pub image: Option<String>, // ??
    #[serde(rename = "type")]
    pub typeval: String,
    pub author_name: Option<String>,
    pub author_url: Option<String>,
    pub provider_name: Option<String>,
    pub provider_url: Option<String>,
    pub html: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Context {
    pub ancestors: Vec<Status>,
    pub descendants: Vec<Status>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Emoji {
    pub shortcode: String,
    pub static_url: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Instance {
    pub uri: String,
    pub title: String,
    pub description: String,
    pub email: String,
    pub version: String,
    pub urls: HashMap<String, String>,
    pub languages: Vec<String>,
    pub contact_account: Option<Account>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mention {
    pub url: String,
    pub username: String,
    pub acct: String,
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    pub id: String,
    #[serde(rename = "type")]
    pub typeval: String,
    pub created_at: String,
    pub account: Account,
    pub status: Option<Status>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relationship {
    pub id: String,
    pub following: bool,
    pub followed_by: bool,
    pub blocking: bool,
    pub muting: bool,
    pub muting_notifications: bool,
    pub requested: bool,
    pub domain_blocking: bool,
    pub showing_reblogs: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Results {
    pub accounts: Vec<Account>,
    pub statuses: Vec<Status>,
    pub hashtags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Status {
    pub id: String,
    pub uri: String,
    pub url: Option<String>,
    pub account: Account,
    pub in_reply_to_id: Option<String>,
    pub in_reply_to_account_id: Option<String>,
    pub reblog: Option<Box<Status>>,
    pub content: String,
    pub created_at: String,
    pub emojis: Vec<Emoji>,
    pub reblogs_count: u32,
    pub favourites_count: u32,
    pub reblogged: Option<bool>,
    pub favourited: Option<bool>,
    pub muted: Option<bool>,
    pub sensitive: bool,
    pub spoiler_text: String,
    pub visibility: String,
    pub media_attachments: Vec<Attachment>,
    pub mentions: Vec<Mention>,
    pub tags: Vec<Tag>,
    pub application: Option<Application>,
    pub language: Option<String>,
    pub pinned: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tag {
    pub name: String,
    pub url: String,
    pub history: Option<Vec<String>>,
}
