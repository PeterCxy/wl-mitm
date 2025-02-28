use std::sync::Arc;

use tracing::{debug, error, info};

use crate::{
    codec::WlRawMsg,
    config::Config,
    objects::WlObjects,
    proto::{
        WL_REGISTRY, WlDisplayDeleteIdEvent, WlDisplayGetRegistryRequest, WlRegistryBindRequest,
        WlRegistryGlobalEvent,
    },
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
    pub fn on_c2s_request(&mut self, raw_msg: &WlRawMsg) -> bool {
        let msg = crate::proto::decode_request(&self.objects, raw_msg);
        if let crate::proto::WaylandProtocolParsingOutcome::MalformedMessage = msg {
            error!(
                obj_id = raw_msg.obj_id,
                opcode = raw_msg.opcode,
                num_fds = raw_msg.fds.len(),
                "Malformed request"
            );
            return false;
        }

        match_decoded! {
            match msg {
                WlDisplayGetRegistryRequest => {
                    self.objects.record_object(WL_REGISTRY, msg.registry);
                }
                WlRegistryBindRequest => {
                    let Some(interface) = self.objects.lookup_global(msg.name) else {
                        return false;
                    };

                    if interface != msg.id_interface_name {
                        error!("Client binding to interface {}, but the interface name {} should correspond to {}", msg.id_interface_name, msg.name, interface);
                        return false;
                    }

                    info!(
                        interface = interface,
                        obj_id = msg.id,
                        "Client binding interface"
                    );

                    if let Some(t) = crate::proto::lookup_known_object_type(interface) {
                        self.objects.record_object(t, msg.id);
                    }
                }
            }
        }

        true
    }

    #[tracing::instrument(skip_all)]
    pub fn on_s2c_event(&mut self, raw_msg: &WlRawMsg) -> bool {
        let msg = crate::proto::decode_event(&self.objects, raw_msg);
        if let crate::proto::WaylandProtocolParsingOutcome::MalformedMessage = msg {
            error!(
                obj_id = raw_msg.obj_id,
                opcode = raw_msg.opcode,
                "Malformed event"
            );
            return false;
        }

        match_decoded! {
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
                WlDisplayDeleteIdEvent => {
                    // When an object is acknowledged to be deleted, remove it from our
                    // internal cache of all registered objects
                    self.objects.remove_object(msg.id);
                }
            }
        }

        true
    }
}
