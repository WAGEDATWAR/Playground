//! Persistence: the .pgsave container, slot manager, migrations, export/import, archive safety, replay
//! logs, bug bundles and scenarios. Blueprint §13, §20.

pub mod archive;
pub mod codec;
pub mod compat;
pub mod crash;
pub mod export;
pub mod logfile;
pub mod migrate;
pub mod scenario;
pub mod store;

#[cfg(test)]
mod fixtures;
#[cfg(test)]
mod testkit;
