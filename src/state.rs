use std::sync::Arc;

use tracing::{debug, info};

use crate::{
    codec::WlRawMsg,
    config::Config,
    objects::WlObjects,
    proto::{WL_REGISTRY, WlDisplayGetRegistry, WlRegistryBind, WlRegistryGlobalEvent},
};

pub struct WlMitmState {
    config: Arc<Config>,
    objects: WlObjects,
}

impl WlMitmState {
    pub fn new(config: Arc<Config>) -> WlMitmState {
        WlMitmState {
            config,
            objects: WlObjects::new(),
        }
    }

    #[tracing::instrument(skip_all)]
    pub fn on_c2s_request(&mut self, msg: &WlRawMsg) -> bool {
        decode_and_match_msg!(
            self.objects,
            match msg {
                WlDisplayGetRegistry => {
                    self.objects.record_object(WL_REGISTRY, msg.registry_new_id);
                }
                WlRegistryBind => {
                    let Some(interface) = self.objects.lookup_global(msg.name) else {
                        return false;
                    };
                    info!(
                        interface = interface,
                        obj_id = msg.new_id,
                        "Client binding interface"
                    );
                }
            }
        );

        true
    }

    #[tracing::instrument(skip_all)]
    pub fn on_s2c_event(&mut self, msg: &WlRawMsg) -> bool {
        decode_and_match_msg!(
            self.objects,
            match msg {
                WlRegistryGlobalEvent => {
                    debug!(
                        interface = msg.interface,
                        name = msg.name,
                        version = msg.version,
                        "got global"
                    );

                    self.objects.record_global(msg.name, msg.interface);

                    if !self.config.filter.allowed_globals.contains(msg.interface) {
                        info!(
                            interface = msg.interface,
                            "Removing interface from published globals"
                        );
                        return false;
                    }
                }
            }
        );

        true
    }
}
