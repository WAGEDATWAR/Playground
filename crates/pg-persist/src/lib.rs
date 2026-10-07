//! Persistence: the .pgsave container, slot manager, migrations, export/import, archive safety, replay
//! logs, bug bundles and scenarios. Blueprint §13, §20.

pub mod archive;
pub mod codec;
pub mod compat;
pub mod export;
pub mod migrate;
pub mod store;

#[cfg(test)]
mod testkit;
