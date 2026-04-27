//! SeaORM entity definitions.
//!
//! Hand-written rather than scaffolded by `sea-orm-cli generate entity`
//! because the schema is small and we want explicit control over enum
//! mapping (channel kind) and the chrono / unix-millis split.

pub mod channel;
pub mod channel_member;
pub mod channel_read;
pub mod file;
pub mod mention;
pub mod message;
pub mod message_file;
pub mod prelude;
pub mod session;
pub mod user;
