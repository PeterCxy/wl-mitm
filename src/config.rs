use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Deserializer};
use serde_derive::Deserialize;

#[derive(Deserialize)]
pub struct Config {
    pub socket: WlSockets,
    pub filter: WlFilter,
}

fn default_upstream_socket() -> String {
    std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "wayland-1".to_string())
}

#[derive(Deserialize)]
pub struct WlSockets {
    listen: String,
    #[serde(default = "default_upstream_socket")]
    upstream: String,
}

impl WlSockets {
    pub fn upstream_socket_path(&self) -> PathBuf {
        let p = Path::new(&self.upstream);
        if p.is_absolute() {
            p.into()
        } else {
            Path::new(
                &std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/run/user/1000".to_string()),
            )
            .join(p)
            .into()
        }
    }

    pub fn listen_socket_path(&self) -> PathBuf {
        let p = Path::new(&self.listen);
        if p.is_absolute() {
            p.into()
        } else {
            Path::new(
                &std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/run/user/1000".to_string()),
            )
            .join(p)
            .into()
        }
    }
}

#[derive(Deserialize)]
pub struct WlFilter {
    pub allowed_globals: HashSet<String>,
    pub ask_cmd: Option<String>,
    #[serde(deserialize_with = "deserialize_filter_requests")]
    pub requests: HashMap<String, Vec<WlFilterRequest>>,
}

#[derive(Deserialize)]
pub enum WlFilterRequestAction {
    #[serde(rename = "block")]
    Block,
    #[serde(rename = "ask")]
    Ask,
}

#[derive(Deserialize)]
pub struct WlFilterRequest {
    pub interface: String,
    pub requests: HashSet<String>,
    pub action: WlFilterRequestAction,
    pub desc: Option<String>,
}

/// Deserialize an array of [WlFilterRequest]s to a hashmap keyed by interface name
pub fn deserialize_filter_requests<'de, D>(
    deserializer: D,
) -> Result<HashMap<String, Vec<WlFilterRequest>>, D::Error>
where
    D: Deserializer<'de>,
{
    let mut map: HashMap<String, Vec<WlFilterRequest>> = HashMap::new();
    for r in Vec::<WlFilterRequest>::deserialize(deserializer)? {
        map.entry(r.interface.clone()).or_default().push(r);
    }
    Ok(map)
}
