use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "messages")]
pub struct Model {
    /// ULID — lexically sortable, so cursor pagination works on PK alone.
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub channel_id: String,
    pub author_id: String,
    pub body: String,
    pub created_at: i64,
    /// `Some(ts)` once the author has edited it at least once.
    pub edited_at: Option<i64>,
    /// Soft-delete tombstone. Rows are never hard-deleted — that lets
    /// `MessageDeleted` events resolve consistently for clients that
    /// missed the live broadcast and re-paginate.
    pub deleted_at: Option<i64>,
    /// Idempotency key supplied by the client. Unique within a channel.
    pub client_msg_id: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::channel::Entity",
        from = "Column::ChannelId",
        to = "super::channel::Column::Id",
        on_delete = "Cascade"
    )]
    Channel,
    #[sea_orm(
        belongs_to = "super::user::Entity",
        from = "Column::AuthorId",
        to = "super::user::Column::Id",
        on_delete = "Cascade"
    )]
    Author,
}

impl Related<super::channel::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Channel.def()
    }
}

impl Related<super::user::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Author.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
