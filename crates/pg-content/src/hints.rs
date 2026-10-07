//! "Did you mean…?" hints for validation messages. The implementation lives in `pg-api` so that content
//! validation and script tooling share one copy.

pub use pg_api::{closest, edit_distance, hint};
