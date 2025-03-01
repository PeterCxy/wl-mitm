use std::sync::Arc;

use tracing::{debug, error, info};

use crate::{
    codec::WlRawMsg,
    config::Config,
    objects::WlObjects,
    proto::{
        WL_REGISTRY, WaylandProtocolParsingOutcome, WlDisplayDeleteIdEvent,
        WlDisplayGetRegistryRequest, WlRegistryBindRequest, WlRegistryGlobalEvent,
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
        let msg = match crate::proto::decode_request(&self.objects, raw_msg) {
            WaylandProtocolParsingOutcome::Ok(msg) => msg,
            WaylandProtocolParsingOutcome::MalformedMessage => {
                // Kill all malformed messages
                // Note that they are different from messages whose object / message types are unknown
                error!(
                    obj_id = raw_msg.obj_id,
                    opcode = raw_msg.opcode,
                    num_fds = raw_msg.fds.len(),
                    "Malformed request"
                );
                return false;
            }
            _ => {
                // Pass through all unknown messages -- they could be from a Wayland protocol we haven't
                // been built against!
                // Note that this won't pass through messages for globals we haven't allowed:
                // to use a global, a client must first _bind_ that global, and _that_ message is intercepted
                // below. There, we match based on the textual representation of the interface, so it works
                // even for globals from protocols we don't know.
                // It does mean we can't filter against methods that create more objects _from_ that
                // global, though.
                return true;
            }
        };

        if let Some(msg) = msg.downcast_ref::<WlDisplayGetRegistryRequest>() {
            self.objects.record_object(WL_REGISTRY, msg.registry);
        } else if let Some(msg) = msg.downcast_ref::<WlRegistryBindRequest>() {
            // If we have blocked this global, this lookup should return None, thus blocking client attempts
            // to bind to a blocked global.
            // Note that because we've removed said global from the registry, a client _SHOULD NOT_ be attempting
            // to bind to it; if it does, it's likely a malicious client!
            // So, we simply remove these messages from the stream, which will cause the Wayland server to error out.
            let Some(interface) = self.objects.lookup_global(msg.name) else {
                return false;
            };

            if interface != msg.id_interface_name {
                error!(
                    "Client binding to interface {}, but the interface name {} should correspond to {}",
                    msg.id_interface_name, msg.name, interface
                );
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

        true
    }

    #[tracing::instrument(skip_all)]
    pub fn on_s2c_event(&mut self, raw_msg: &WlRawMsg) -> bool {
        let msg = match crate::proto::decode_request(&self.objects, raw_msg) {
            WaylandProtocolParsingOutcome::Ok(msg) => msg,
            WaylandProtocolParsingOutcome::MalformedMessage => {
                error!(
                    obj_id = raw_msg.obj_id,
                    opcode = raw_msg.opcode,
                    num_fds = raw_msg.fds.len(),
                    "Malformed event"
                );
                return false;
            }
            _ => {
                return true;
            }
        };

        if let Some(msg) = msg.downcast_ref::<WlRegistryGlobalEvent>() {
            // This event is how Wayland servers announce globals -- and they are the entrypoint to
            // most extensions! You need at least one global registered for clients to be able to
            // access methods from that extension; but those methods _could_ create more objects.
            debug!(
                interface = msg.interface,
                name = msg.name,
                version = msg.version,
                "got global"
            );

            // To block entire extensions, we just need to filter out their announced global objects.
            if !self.config.filter.allowed_globals.contains(msg.interface) {
                info!(
                    interface = msg.interface,
                    "Removing interface from published globals"
                );
                return false;
            }

            // Else, record the global object. These are the only ones we're ever going to allow through.
            // We block bind requests on any interface that's not recorded here.
            self.objects.record_global(msg.name, msg.interface);
        } else if let Some(msg) = msg.downcast_ref::<WlDisplayDeleteIdEvent>() {
            // Server has acknowledged deletion of an object
            self.objects.remove_object(msg.id);
        }

        true
    }
}
