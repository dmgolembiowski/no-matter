use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Users
        manager
            .create_table(
                Table::create()
                    .table(Users::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(Users::Id).text().not_null().primary_key())
                    .col(
                        ColumnDef::new(Users::Username)
                            .text()
                            .not_null()
                            .unique_key(),
                    )
                    .col(ColumnDef::new(Users::PasswordHash).text().not_null())
                    .col(ColumnDef::new(Users::CreatedAt).big_integer().not_null())
                    .to_owned(),
            )
            .await?;

        // Sessions
        manager
            .create_table(
                Table::create()
                    .table(Sessions::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(Sessions::Id).text().not_null().primary_key())
                    .col(ColumnDef::new(Sessions::UserId).text().not_null())
                    .col(
                        ColumnDef::new(Sessions::TokenHash)
                            .text()
                            .not_null()
                            .unique_key(),
                    )
                    .col(ColumnDef::new(Sessions::ExpiresAt).big_integer().not_null())
                    .col(ColumnDef::new(Sessions::CreatedAt).big_integer().not_null())
                    .foreign_key(
                        ForeignKey::create()
                            .from(Sessions::Table, Sessions::UserId)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_sessions_token_hash")
                    .table(Sessions::Table)
                    .col(Sessions::TokenHash)
                    .to_owned(),
            )
            .await?;

        // Channels
        manager
            .create_table(
                Table::create()
                    .table(Channels::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(Channels::Id).text().not_null().primary_key())
                    .col(ColumnDef::new(Channels::Name).text().not_null())
                    .col(ColumnDef::new(Channels::Kind).text().not_null())
                    .col(ColumnDef::new(Channels::CreatedAt).big_integer().not_null())
                    .check(Expr::col(Channels::Kind).is_in(["public", "private", "dm", "group"]))
                    .to_owned(),
            )
            .await?;

        // Channel name uniqueness is case-insensitive, scoped to the
        // public/private/group kinds. DMs share a name space with
        // themselves only (a DM's "name" is a synthetic display string,
        // not user-chosen) — we exclude them via a partial index.
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE UNIQUE INDEX IF NOT EXISTS idx_channels_name_unique \
                 ON channels (lower(name)) \
                 WHERE kind != 'dm'",
            )
            .await?;

        // Channel members
        manager
            .create_table(
                Table::create()
                    .table(ChannelMembers::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(ChannelMembers::ChannelId).text().not_null())
                    .col(ColumnDef::new(ChannelMembers::UserId).text().not_null())
                    .col(
                        ColumnDef::new(ChannelMembers::JoinedAt)
                            .big_integer()
                            .not_null(),
                    )
                    .primary_key(
                        Index::create()
                            .col(ChannelMembers::ChannelId)
                            .col(ChannelMembers::UserId),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .from(ChannelMembers::Table, ChannelMembers::ChannelId)
                            .to(Channels::Table, Channels::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .from(ChannelMembers::Table, ChannelMembers::UserId)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_channel_members_user")
                    .table(ChannelMembers::Table)
                    .col(ChannelMembers::UserId)
                    .to_owned(),
            )
            .await?;

        // Messages
        manager
            .create_table(
                Table::create()
                    .table(Messages::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(Messages::Id).text().not_null().primary_key())
                    .col(ColumnDef::new(Messages::ChannelId).text().not_null())
                    .col(ColumnDef::new(Messages::AuthorId).text().not_null())
                    .col(ColumnDef::new(Messages::Body).text().not_null())
                    .col(ColumnDef::new(Messages::CreatedAt).big_integer().not_null())
                    .col(ColumnDef::new(Messages::EditedAt).big_integer().null())
                    .col(ColumnDef::new(Messages::DeletedAt).big_integer().null())
                    .col(ColumnDef::new(Messages::ClientMsgId).text().null())
                    .foreign_key(
                        ForeignKey::create()
                            .from(Messages::Table, Messages::ChannelId)
                            .to(Channels::Table, Channels::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .from(Messages::Table, Messages::AuthorId)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        // Pagination workhorse: (channel_id, id DESC) — backend.md §6.
        manager
            .create_index(
                Index::create()
                    .name("idx_messages_channel_id_desc")
                    .table(Messages::Table)
                    .col(Messages::ChannelId)
                    .col((Messages::Id, IndexOrder::Desc))
                    .to_owned(),
            )
            .await?;

        // Idempotency: same client_msg_id within a channel collapses.
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE UNIQUE INDEX IF NOT EXISTS idx_messages_client_msg_id \
                 ON messages (channel_id, client_msg_id) \
                 WHERE client_msg_id IS NOT NULL",
            )
            .await?;

        // Channel reads (last-read pointer per user per channel).
        manager
            .create_table(
                Table::create()
                    .table(ChannelReads::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(ChannelReads::UserId).text().not_null())
                    .col(ColumnDef::new(ChannelReads::ChannelId).text().not_null())
                    .col(
                        ColumnDef::new(ChannelReads::LastReadMessageId)
                            .text()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ChannelReads::ReadAt)
                            .big_integer()
                            .not_null(),
                    )
                    .primary_key(
                        Index::create()
                            .col(ChannelReads::UserId)
                            .col(ChannelReads::ChannelId),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .from(ChannelReads::Table, ChannelReads::UserId)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .from(ChannelReads::Table, ChannelReads::ChannelId)
                            .to(Channels::Table, Channels::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        // Files (most fields exist for forward-compat; current handlers
        // return 501 for the upload + meta endpoints).
        manager
            .create_table(
                Table::create()
                    .table(Files::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(Files::Id).text().not_null().primary_key())
                    .col(ColumnDef::new(Files::Name).text().not_null())
                    .col(ColumnDef::new(Files::Mime).text().not_null())
                    .col(ColumnDef::new(Files::Size).big_integer().not_null())
                    .col(ColumnDef::new(Files::StorageKey).text().not_null())
                    .col(ColumnDef::new(Files::ThumbUrl).text().null())
                    .col(ColumnDef::new(Files::Status).text().not_null())
                    .col(ColumnDef::new(Files::UploaderId).text().not_null())
                    .col(ColumnDef::new(Files::CreatedAt).big_integer().not_null())
                    .check(Expr::col(Files::Status).is_in(["pending_thumb", "ready"]))
                    .foreign_key(
                        ForeignKey::create()
                            .from(Files::Table, Files::UploaderId)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        // Message ↔ file join.
        manager
            .create_table(
                Table::create()
                    .table(MessageFiles::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(MessageFiles::MessageId).text().not_null())
                    .col(ColumnDef::new(MessageFiles::FileId).text().not_null())
                    .col(ColumnDef::new(MessageFiles::Position).integer().not_null())
                    .primary_key(
                        Index::create()
                            .col(MessageFiles::MessageId)
                            .col(MessageFiles::FileId),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .from(MessageFiles::Table, MessageFiles::MessageId)
                            .to(Messages::Table, Messages::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .from(MessageFiles::Table, MessageFiles::FileId)
                            .to(Files::Table, Files::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        // Mentions (forward-compat: server-side mention parsing isn't
        // wired yet, but the table is here so we can backfill cheaply).
        manager
            .create_table(
                Table::create()
                    .table(Mentions::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(Mentions::MessageId).text().not_null())
                    .col(ColumnDef::new(Mentions::UserId).text().not_null())
                    .primary_key(
                        Index::create()
                            .col(Mentions::MessageId)
                            .col(Mentions::UserId),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .from(Mentions::Table, Mentions::MessageId)
                            .to(Messages::Table, Messages::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .from(Mentions::Table, Mentions::UserId)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_mentions_user")
                    .table(Mentions::Table)
                    .col(Mentions::UserId)
                    .col(Mentions::MessageId)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Order matters — drop tables holding foreign keys first.
        for tbl in [
            Mentions::Table.to_string(),
            MessageFiles::Table.to_string(),
            Files::Table.to_string(),
            ChannelReads::Table.to_string(),
            Messages::Table.to_string(),
            ChannelMembers::Table.to_string(),
            Channels::Table.to_string(),
            Sessions::Table.to_string(),
            Users::Table.to_string(),
        ] {
            manager
                .drop_table(Table::drop().table(Alias::new(&tbl)).to_owned())
                .await?;
        }
        Ok(())
    }
}

#[derive(Iden)]
enum Users {
    Table,
    Id,
    Username,
    PasswordHash,
    CreatedAt,
}

#[derive(Iden)]
enum Sessions {
    Table,
    Id,
    UserId,
    TokenHash,
    ExpiresAt,
    CreatedAt,
}

#[derive(Iden)]
enum Channels {
    Table,
    Id,
    Name,
    Kind,
    CreatedAt,
}

#[derive(Iden)]
enum ChannelMembers {
    Table,
    ChannelId,
    UserId,
    JoinedAt,
}

#[derive(Iden)]
enum Messages {
    Table,
    Id,
    ChannelId,
    AuthorId,
    Body,
    CreatedAt,
    EditedAt,
    DeletedAt,
    ClientMsgId,
}

#[derive(Iden)]
enum ChannelReads {
    Table,
    UserId,
    ChannelId,
    LastReadMessageId,
    ReadAt,
}

#[derive(Iden)]
enum Files {
    Table,
    Id,
    Name,
    Mime,
    Size,
    StorageKey,
    ThumbUrl,
    Status,
    UploaderId,
    CreatedAt,
}

#[derive(Iden)]
enum MessageFiles {
    Table,
    MessageId,
    FileId,
    Position,
}

#[derive(Iden)]
enum Mentions {
    Table,
    MessageId,
    UserId,
}
