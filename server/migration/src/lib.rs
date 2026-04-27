//! Migration crate.
//!
//! Migrations are listed in chronological order by timestamp prefix.
//! Add new ones by creating a `m{date}_{seq}_{name}.rs` module and
//! pushing it onto `migrations()` below — sea-orm-migration runs them
//! by `MigrationName::name()`, which is just the filename.

pub use sea_orm_migration::prelude::*;

mod m20260101_000001_init;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(m20260101_000001_init::Migration)]
    }
}
