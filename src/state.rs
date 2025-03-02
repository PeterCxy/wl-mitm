use std::sync::Arc;

use tracing::{debug, error, info, warn};

use crate::{
    codec::WlRawMsg,
    config::{Config, WlFilterRequestAction},
    objects::WlObjects,
    proto::{
        AnyWlParsedMessage, WaylandProtocolParsingOutcome, WlDisplayDeleteIdEvent,
        WlRegistryBindRequest, WlRegistryGlobalEvent, WlRegistryGlobalRemoveEvent,
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

    /// Handle messages which register new objects with known interfaces or deletes them.
    ///
    /// Note that most _globals_ are instantiated using [WlRegistryBindRequest]. That request
    /// is not handled here.
    fn handle_created_or_destroyed_objects(&mut self, msg: &dyn AnyWlParsedMessage<'_>) {
        if let Some(created_objects) = msg.known_objects_created() {
            if let Some(parent_obj) = self.objects.lookup_object(msg.obj_id()) {
                for (id, tt) in created_objects.into_iter() {
                    debug!(
                        parent_obj_id = msg.obj_id(),
                        obj_type = tt.interface(),
                        obj_id = id,
                        "Created object via message {}::{}",
                        parent_obj.interface(),
                        msg.self_msg_name()
                    );
                    self.objects.record_object(tt, id);
                }
            } else {
                error!("Parent object ID {} not found, ignoring", msg.obj_id());
            }
        } else if msg.is_destructor() {
            if let Some(obj_type) = self.objects.lookup_object(msg.obj_id()) {
                debug!(
                    obj_id = msg.obj_id(),
                    "Object destructed via destructor {}::{}",
                    obj_type.interface(),
                    msg.self_msg_name()
                );
            }

            self.objects.remove_object(msg.obj_id());
        }
    }

    /// Returns the number of fds consumed while parsing the message as a concrete Wayland type, and a verdict
    #[tracing::instrument(skip_all)]
    pub async fn on_c2s_request(&mut self, raw_msg: &WlRawMsg) -> (usize, bool) {
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
                return (0, false);
            }
            _ => {
                // TODO: due to fds, we can't expect to be able to pass through unknown messages.
                // Load all sensible Wayland extensions and remove this condition.
                // Pass through all unknown messages -- they could be from a Wayland protocol we haven't
                // been built against!
                // Note that this won't pass through messages for globals we haven't allowed:
                // to use a global, a client must first _bind_ that global, and _that_ message is intercepted
                // below. There, we match based on the textual representation of the interface, so it works
                // even for globals from protocols we don't know.
                // It does mean we can't filter against methods that create more objects _from_ that
                // global, though.
                return (0, true);
            }
        };

        self.handle_created_or_destroyed_objects(&*msg);

        // The bind request doesn't create interface with a fixed type; handle it separately.
        if let Some(msg) = msg.downcast_ref::<WlRegistryBindRequest>() {
            // If we have blocked this global, this lookup should return None, thus blocking client attempts
            // to bind to a blocked global.
            // Note that because we've removed said global from the registry, a client _SHOULD NOT_ be attempting
            // to bind to it; if it does, it's likely a malicious client!
            // So, we simply remove these messages from the stream, which will cause the Wayland server to error out.
            let Some(interface) = self.objects.lookup_global(msg.name) else {
                return (0, false);
            };

            if interface != msg.id_interface_name {
                error!(
                    "Client binding to interface {}, but the interface name {} should correspond to {}",
                    msg.id_interface_name, msg.name, interface
                );
                return (0, false);
            }

            info!(
                interface = interface,
                version = msg.id_interface_version,
                obj_id = msg.id,
                "Client binding interface"
            );

            if let Some(t) = crate::proto::lookup_known_object_type(interface) {
                self.objects.record_object(t, msg.id);
            }
        }

        // Handle requests configured to be filtered
        if let Some(filtered_requests) = self
            .config
            .filter
            .requests
            .get(msg.self_object_type().interface())
        {
            if let Some(filtered) = filtered_requests
                .iter()
                .find(|f| f.requests.contains(msg.self_msg_name()))
            {
                match filtered.action {
                    WlFilterRequestAction::Ask => {
                        if let Some(ref ask_cmd) = self.config.filter.ask_cmd {
                            info!(
                                ask_cmd = ask_cmd,
                                "Running ask command for {}::{}",
                                msg.self_object_type().interface(),
                                msg.self_msg_name()
                            );

                            let mut cmd = tokio::process::Command::new(ask_cmd);
                            cmd.arg(msg.self_object_type().interface());
                            cmd.arg(msg.self_msg_name());
                            // Note: the _last_ argument is always the JSON representation!
                            cmd.arg(msg.to_json());

                            if let Ok(status) = cmd.status().await {
                                if !status.success() {
                                    warn!(
                                        "Blocked {}::{} because of return status {}",
                                        msg.self_object_type().interface(),
                                        msg.self_msg_name(),
                                        status
                                    );
                                }

                                return (msg.num_consumed_fds(), status.success());
                            }
                        }

                        warn!(
                            "Blocked {}::{} because of missing ask_cmd",
                            msg.self_object_type().interface(),
                            msg.self_msg_name()
                        );
                        return (msg.num_consumed_fds(), false);
                    }
                    WlFilterRequestAction::Block => {
                        warn!(
                            "Blocked {}::{}",
                            msg.self_object_type().interface(),
                            msg.self_msg_name()
                        );
                        // TODO: don't just return false, build an error event
                        return (msg.num_consumed_fds(), false);
                    }
                }
            }
        }

        (msg.num_consumed_fds(), true)
    }

    #[tracing::instrument(skip_all)]
    pub async fn on_s2c_event(&mut self, raw_msg: &WlRawMsg) -> (usize, bool) {
        let msg = match crate::proto::decode_event(&self.objects, raw_msg) {
            WaylandProtocolParsingOutcome::Ok(msg) => msg,
            WaylandProtocolParsingOutcome::MalformedMessage => {
                error!(
                    obj_id = raw_msg.obj_id,
                    opcode = raw_msg.opcode,
                    num_fds = raw_msg.fds.len(),
                    "Malformed event"
                );
                return (0, false);
            }
            _ => {
                return (0, true);
            }
        };

        self.handle_created_or_destroyed_objects(&*msg);

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
                return (0, false);
            }

            // Else, record the global object. These are the only ones we're ever going to allow through.
            // We block bind requests on any interface that's not recorded here.
            self.objects.record_global(msg.name, msg.interface);
        } else if let Some(msg) = msg.downcast_ref::<WlRegistryGlobalRemoveEvent>() {
            // Remove globals that the server has removed
            self.objects.remove_global(msg.name);
        } else if let Some(msg) = msg.downcast_ref::<WlDisplayDeleteIdEvent>() {
            // Server has acknowledged deletion of an object
            self.objects.remove_object(msg.id);
        }

        (msg.num_consumed_fds(), true)
    }
}
