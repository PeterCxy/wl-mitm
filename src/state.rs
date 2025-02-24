use tracing::{debug, info};

use crate::{
    codec::WlRawMsg,
    objects::{WlObjectType, WlObjects},
    proto::{WlDisplayGetRegistry, WlRegistryBind, WlRegistryGlobalEvent},
};

pub struct WlMitmState {
    objects: WlObjects,
}

impl WlMitmState {
    pub fn new() -> WlMitmState {
        WlMitmState {
            objects: WlObjects::new(),
        }
    }

    pub fn on_c2s_request(&mut self, msg: &WlRawMsg) -> bool {
        decode_and_match_msg!(
            self.objects,
            match msg {
                WlDisplayGetRegistry => {
                    self.objects
                        .record_object(WlObjectType::WlRegistry, msg.registry_new_id);
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
                }
            }
        );

        true
    }
}
