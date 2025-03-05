use std::sync::Arc;

use tracing::{debug, error, info, warn};

use crate::{
    codec::WlRawMsg,
    config::{Config, WlFilterRequestAction, WlFilterRequestBlockType},
    objects::WlObjects,
    proto::{
        AnyWlParsedMessage, WaylandProtocolParsingOutcome, WlDisplayDeleteIdEvent,
        WlRegistryBindRequest, WlRegistryGlobalEvent, WlRegistryGlobalRemoveEvent,
    },
};

/// What to do for a message?
pub enum WlMitmVerdict {
    /// This message is allowed. Pass it through to the opposite end.
    Allowed,
    /// This message is filtered
    Filtered,
    /// This messages is rejected (i.e. filtered, but comes with an error code to return to sender)
    Rejected(u32),
    /// Terminate this entire session. Something is off.
    Terminate,
}

impl Default for WlMitmVerdict {
    fn default() -> Self {
        WlMitmVerdict::Terminate
    }
}

/// Result returned by [WlMitmState] when handling messages.
/// It's a pair of (num_consumed_fds, verdict).
///
/// We need to return back unused fds from the [WlRawMsg], which
/// is why this has to be returned from here.
#[derive(Default)]
pub struct WlMitmOutcome(pub usize, pub WlMitmVerdict);

impl WlMitmOutcome {
    fn set_consumed_fds(&mut self, consumed_fds: usize) {
        self.0 = consumed_fds;
    }

    fn allowed(mut self) -> Self {
        self.1 = WlMitmVerdict::Allowed;
        self
    }

    fn filtered(mut self) -> Self {
        self.1 = WlMitmVerdict::Filtered;
        self
    }

    fn terminate(mut self) -> Self {
        self.1 = WlMitmVerdict::Terminate;
        self
    }

    fn rejected(mut self, error_code: u32) -> Self {
        self.1 = WlMitmVerdict::Rejected(error_code);
        self
    }
}

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
    /// If there is an error, this function will return false and the connection shall be terminated.
    ///
    /// Note that most _globals_ are instantiated using [WlRegistryBindRequest]. That request
    /// is not handled here.
    fn handle_created_or_destroyed_objects(
        &mut self,
        msg: &dyn AnyWlParsedMessage<'_>,
        from_client: bool,
    ) -> bool {
        if let Some(created_objects) = msg.known_objects_created() {
            if let Some(parent_obj) = self.objects.lookup_object(msg.obj_id()) {
                for (id, tt) in created_objects.into_iter() {
                    if let Some(existing_obj_type) = self.objects.lookup_object(id) {
                        debug!(
                            parent_obj_id = msg.obj_id(),
                            obj_type = tt.interface(),
                            obj_id = id,
                            existing_obj_type = existing_obj_type.interface(),
                            is_half_destroyed = self.objects.is_half_destroyed(id),
                            "Trying to create object via message {}::{} but the object ID is already used!",
                            parent_obj.interface(),
                            msg.self_msg_name()
                        );

                        return false;
                    }

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
                error!("Parent object ID {} not found!", msg.obj_id());
                return false;
            }
        } else if msg.is_destructor() {
            let Some(obj_type) = self.objects.lookup_object(msg.obj_id()) else {
                // This shouldn't really happen -- to decode the message we have to have a record of the object
                error!("Destructed object ID {} not found!", msg.obj_id());
                return false;
            };

            debug!(
                obj_id = msg.obj_id(),
                "Object destructed via destructor {}::{}",
                obj_type.interface(),
                msg.self_msg_name()
            );

            self.objects.remove_object(msg.obj_id(), from_client);
        }

        true
    }

    /// Returns the number of fds consumed while parsing the message as a concrete Wayland type, and a verdict
    #[tracing::instrument(skip_all)]
    pub async fn on_c2s_request(&mut self, raw_msg: &WlRawMsg) -> WlMitmOutcome {
        let mut outcome: WlMitmOutcome = Default::default();
        let msg = match crate::proto::decode_request(&self.objects, raw_msg) {
            WaylandProtocolParsingOutcome::Ok(msg) => msg,
            _ => {
                let obj_type = self
                    .objects
                    .lookup_object(raw_msg.obj_id)
                    .map(|t| t.interface());

                error!(
                    obj_id = raw_msg.obj_id,
                    obj_type = ?obj_type,
                    opcode = raw_msg.opcode,
                    num_fds = raw_msg.fds.len(),
                    "Malformed or unknown request"
                );
                return outcome.terminate();
            }
        };

        outcome.set_consumed_fds(msg.num_consumed_fds());

        // To get here, the object referred to in raw_msg must exist, but it might already be destroyed by the client
        // In that case, the client is broken!
        if self.objects.is_half_destroyed(msg.obj_id()) {
            error!(
                obj_id = msg.obj_id(),
                opcode = msg.self_opcode(),
                "Client request detected on object already scheduled for destruction; aborting!"
            );
            return outcome.terminate();
        }

        if !self.handle_created_or_destroyed_objects(&*msg, true) {
            return outcome.terminate();
        }

        // The bind request doesn't create interface with a fixed type; handle it separately.
        if let Some(msg) = msg.downcast_ref::<WlRegistryBindRequest>() {
            // If we have blocked this global, this lookup should return None, thus blocking client attempts
            // to bind to a blocked global.
            // Note that because we've removed said global from the registry, a client _SHOULD NOT_ be attempting
            // to bind to it; if it does, it's likely a malicious client!
            // So, we simply remove these messages from the stream, which will cause the Wayland server to error out.
            let Some(obj_type) = self.objects.lookup_global(msg.name) else {
                return outcome.terminate();
            };

            if obj_type.interface() != msg.id_interface_name {
                error!(
                    "Client binding to interface {}, but the interface name {} should correspond to {}",
                    msg.id_interface_name,
                    msg.name,
                    obj_type.interface()
                );
                return outcome.terminate();
            }

            info!(
                interface = obj_type.interface(),
                version = msg.id_interface_version,
                obj_id = msg.id,
                "Client binding interface"
            );

            self.objects.record_object(obj_type, msg.id);
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
                            cmd.arg(filtered.desc.as_deref().unwrap_or_else(|| ""));
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

                                    return match filtered.block_type {
                                        WlFilterRequestBlockType::Ignore => outcome.filtered(),
                                        WlFilterRequestBlockType::Reject => {
                                            outcome.rejected(filtered.error_code)
                                        }
                                    };
                                } else {
                                    return outcome.allowed();
                                }
                            }
                        }

                        warn!(
                            "Blocked {}::{} because of missing ask_cmd",
                            msg.self_object_type().interface(),
                            msg.self_msg_name()
                        );
                        return match filtered.block_type {
                            WlFilterRequestBlockType::Ignore => outcome.filtered(),
                            WlFilterRequestBlockType::Reject => {
                                outcome.rejected(filtered.error_code)
                            }
                        };
                    }
                    WlFilterRequestAction::Block => {
                        warn!(
                            "Blocked {}::{}",
                            msg.self_object_type().interface(),
                            msg.self_msg_name()
                        );
                        return match filtered.block_type {
                            WlFilterRequestBlockType::Ignore => outcome.filtered(),
                            WlFilterRequestBlockType::Reject => {
                                outcome.rejected(filtered.error_code)
                            }
                        };
                    }
                }
            }
        }

        outcome.allowed()
    }

    #[tracing::instrument(skip_all)]
    pub async fn on_s2c_event(&mut self, raw_msg: &WlRawMsg) -> WlMitmOutcome {
        let mut outcome: WlMitmOutcome = Default::default();
        let msg = match crate::proto::decode_event(&self.objects, raw_msg) {
            WaylandProtocolParsingOutcome::Ok(msg) => msg,
            _ => {
                let obj_type = self
                    .objects
                    .lookup_object(raw_msg.obj_id)
                    .map(|t| t.interface());

                error!(
                    obj_id = raw_msg.obj_id,
                    obj_type = ?obj_type,
                    opcode = raw_msg.opcode,
                    num_fds = raw_msg.fds.len(),
                    "Malformed or unknown event"
                );
                return outcome.terminate();
            }
        };

        outcome.set_consumed_fds(msg.num_consumed_fds());

        if !self.handle_created_or_destroyed_objects(&*msg, false) {
            return outcome.terminate();
        }

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

            let Some(obj_type) = crate::proto::lookup_known_object_type(msg.interface) else {
                error!(
                    interface = msg.interface,
                    "Unknown interface removed! If required, please include its XML when building wl-mitm!"
                );

                return outcome.filtered();
            };

            // To block entire extensions, we just need to filter out their announced global objects.
            if !self.config.filter.allowed_globals.contains(msg.interface) {
                info!(
                    interface = msg.interface,
                    "Removing interface from published globals"
                );
                return outcome.filtered();
            }

            // Else, record the global object. These are the only ones we're ever going to allow through.
            // We block bind requests on any interface that's not recorded here.
            self.objects.record_global(msg.name, obj_type);
        } else if let Some(msg) = msg.downcast_ref::<WlRegistryGlobalRemoveEvent>() {
            // Remove globals that the server has removed
            self.objects.remove_global(msg.name);
        } else if let Some(msg) = msg.downcast_ref::<WlDisplayDeleteIdEvent>() {
            // Server has acknowledged deletion of an object
            self.objects.remove_object(msg.id, false);
        }

        outcome.allowed()
    }
}
