//! One module per source; each exposes `pub async fn run(state: State, opts: toml::Table)` that loops forever and never panics.
pub mod docker;
pub mod gateway;
pub mod host;
pub mod litellm;
pub mod ollama;
pub mod traefik;
