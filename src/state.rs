use std::sync::Arc;

use tracing::{debug, error, info, warn};

use crate::{
    codec::WlRawMsg,
    config::{Config, WlFilterRequestAction, WlFilterRequestBlockType},
    objects::WlObjects,
    proto::{
        AnyWlParsedMessage, WaylandProtocolParsingOutcome, WlDisplayDeleteIdEvent,
        WlKeyboardEnterEvent, WlParsedMessage, WlPointerEnterEvent, WlRegistryBindRequest,
        WlRegistryGlobalEvent, WlRegistryGlobalRemoveEvent, WlTouchDownEvent,
        XdgSurfaceGetToplevelRequest, XdgToplevelSetAppIdRequest, XdgToplevelSetTitleRequest,
        XdgWmBaseGetXdgSurfaceRequest,
    },
};

/// What to do for a message?
#[derive(Debug)]
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

impl WlMitmVerdict {
    pub fn is_allowed(&self) -> bool {
        matches!(self, WlMitmVerdict::Allowed)
    }
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

/// Association between a wl_surface and an xdg_surface, to facilitate
/// lookup for [ToplevelSurfaceInfo] from a wl_surface
struct SurfaceXdgAssociation(u32);
/// Association between an xdg_surface and an xdg_toplevel
struct XdgToplevelAssociation(u32);

/// A struct to track information about an app's top-level surfaces (windows)
/// This gets passed down to ask and notify scripts to produce user-friendly
/// messages.
#[derive(Default, Debug)]
struct ToplevelSurfaceInfo {
    pub title: Option<String>,
    pub app_id: Option<String>,
}

/// Tracks state for _one_ Wayland connection.
pub struct WlMitmState {
    config: Arc<Config>,
    objects: WlObjects,
    /// The last toplevel object ID (NOT the underlying wl_surface) that was "active"
    /// for this connection.
    /// This is used to hint the ask and notify scripts about the app's id and name,
    /// even though this can never actually be perfect -- we can't track precisely
    /// what might have caused the last filtered request to happen!
    last_toplevel: Option<u32>,
}

impl WlMitmState {
    pub fn new(config: Arc<Config>) -> WlMitmState {
        WlMitmState {
            config,
            objects: WlObjects::new(),
            last_toplevel: None,
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

            if self.last_toplevel.is_some_and(|id| id == msg.obj_id()) {
                self.last_toplevel = None;
            }
        }

        true
    }

    fn prepare_command(
        &self,
        msg: &dyn AnyWlParsedMessage<'_>,
        cmd_str: &str,
        desc: &str,
    ) -> tokio::process::Command {
        let mut cmd = tokio::process::Command::new(cmd_str);
        cmd.arg(msg.self_object_type().interface());
        cmd.arg(msg.self_msg_name());
        cmd.arg(desc);
        cmd.env("WL_MITM_MSG_JSON", msg.to_json());

        if let Some(last_toplevel) = self.last_toplevel {
            if let Some(info) = self
                .objects
                .get_object_extension::<ToplevelSurfaceInfo>(last_toplevel)
            {
                if let Some(ref title) = info.title {
                    cmd.env("WL_MITM_LAST_TOPLEVEL_TITLE", title);
                }

                if let Some(ref app_id) = info.app_id {
                    cmd.env("WL_MITM_LAST_TOPLEVEL_APP_ID", app_id);
                }
            }
        }

        cmd
    }

    fn update_last_active_surface(&mut self, surface: u32) {
        if let Some(SurfaceXdgAssociation(xdg_surface)) = self.objects.get_object_extension(surface)
        {
            if let Some(XdgToplevelAssociation(xdg_toplevel)) =
                self.objects.get_object_extension(*xdg_surface)
            {
                self.last_toplevel = Some(*xdg_toplevel);
            }
        }
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

        if self.config.logging.log_all_requests {
            debug!(
                obj_id = msg.obj_id(),
                raw_payload_bytes = ?raw_msg.payload(),
                num_fds = raw_msg.fds.len(),
                num_consumed_fds = msg.num_consumed_fds(),
                "{}::{}",
                msg.self_object_type().interface(),
                msg.self_msg_name(),
            )
        }

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
                warn!(
                    interface = msg.name,
                    version = msg.id_interface_version,
                    obj_id = msg.id,
                    "Client binding non-existent or filtered interface"
                );
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
        } else if let Some(msg) = msg.downcast_ref::<XdgWmBaseGetXdgSurfaceRequest>() {
            self.objects
                .put_object_extension(msg.surface, SurfaceXdgAssociation(msg.id));
        } else if let Some(msg) = msg.downcast_ref::<XdgSurfaceGetToplevelRequest>() {
            self.objects
                .put_object_extension(msg.obj_id(), XdgToplevelAssociation(msg.id));
            self.objects
                .put_object_extension(msg.id, ToplevelSurfaceInfo::default());
        } else if let Some(msg) = msg.downcast_ref::<XdgToplevelSetAppIdRequest>() {
            if let Some(info) = self
                .objects
                .get_object_extension_mut::<ToplevelSurfaceInfo>(msg.obj_id())
            {
                info.app_id = Some(msg.app_id.to_string());
            }
        } else if let Some(msg) = msg.downcast_ref::<XdgToplevelSetTitleRequest>() {
            if let Some(info) = self
                .objects
                .get_object_extension_mut::<ToplevelSurfaceInfo>(msg.obj_id())
            {
                info.title = Some(msg.title.to_string());
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
                        if let Some(ref ask_cmd) = self.config.exec.ask_cmd {
                            info!(
                                ask_cmd = ask_cmd,
                                "Running ask command for {}::{}",
                                msg.self_object_type().interface(),
                                msg.self_msg_name()
                            );

                            let mut cmd = self.prepare_command(
                                &*msg,
                                ask_cmd,
                                filtered.desc.as_deref().unwrap_or_else(|| ""),
                            );

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
                    WlFilterRequestAction::Notify => {
                        if let Some(ref notify_cmd) = self.config.exec.notify_cmd {
                            info!(
                                notify_cmd = notify_cmd,
                                "Running notify command for {}::{}",
                                msg.self_object_type().interface(),
                                msg.self_msg_name()
                            );

                            let mut cmd = self.prepare_command(
                                &*msg,
                                notify_cmd,
                                filtered.desc.as_deref().unwrap_or_else(|| ""),
                            );

                            cmd.spawn().ok();
                        }
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

        if self.config.logging.log_all_events {
            debug!(
                obj_id = msg.obj_id(),
                raw_payload_bytes = ?raw_msg.payload(),
                num_fds = raw_msg.fds.len(),
                num_consumed_fds = msg.num_consumed_fds(),
                "{}::{}",
                msg.self_object_type().interface(),
                msg.self_msg_name(),
            )
        }

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
        } else if let Some(msg) = msg.downcast_ref::<WlPointerEnterEvent>() {
            self.update_last_active_surface(msg.surface);
        } else if let Some(msg) = msg.downcast_ref::<WlKeyboardEnterEvent>() {
            self.update_last_active_surface(msg.surface);
        } else if let Some(msg) = msg.downcast_ref::<WlTouchDownEvent>() {
            self.update_last_active_surface(msg.surface);
        }

        outcome.allowed()
    }
}
