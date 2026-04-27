use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// Channel kind, stored in the `kind` text column. The CHECK constraint
/// in the migration restricts to these four values; an unknown value in
/// the DB is treated as a hard error rather than silently coerced.
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize, Deserialize)]
#[sea_orm(rs_type = "String", db_type = "String(StringLen::None)")]
#[serde(rename_all = "snake_case")]
pub enum ChannelKind {
    #[sea_orm(string_value = "public")]
    Public,
    #[sea_orm(string_value = "private")]
    Private,
    #[sea_orm(string_value = "dm")]
    Dm,
    #[sea_orm(string_value = "group")]
    Group,
}

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "channels")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub name: String,
    pub kind: ChannelKind,
    pub created_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::message::Entity")]
    Messages,
    #[sea_orm(has_many = "super::channel_member::Entity")]
    Members,
}

impl Related<super::message::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Messages.def()
    }
}

impl Related<super::channel_member::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Members.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
