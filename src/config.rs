use std::path::{Path, PathBuf};

use serde_derive::Deserialize;

#[derive(Deserialize)]
pub struct Config {
    pub socket: WlSockets,
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
