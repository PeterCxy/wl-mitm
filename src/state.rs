use crate::{
    codec::WlRawMsg,
    proto::{WaylandProtocolParsingOutcome, WlDisplayGetRegistry, WlRegistryGlobalEvent},
};

macro_rules! reject_malformed {
    ($e:expr) => {
        if let WaylandProtocolParsingOutcome::MalformedMessage = $e {
            return false;
        } else if let WaylandProtocolParsingOutcome::Ok(e) = $e {
            Some(e)
        } else {
            None
        }
    };
}

pub struct WlMitmState {
    registry_obj_id: Option<u32>,
}

impl WlMitmState {
    pub fn new() -> WlMitmState {
        WlMitmState {
            registry_obj_id: None,
        }
    }

    pub fn on_c2s_msg(&mut self, msg: &WlRawMsg) -> bool {
        if self.registry_obj_id.is_none() {
            if let Some(get_registry_msg) =
                reject_malformed!(WlDisplayGetRegistry::try_from_msg(msg))
            {
                self.registry_obj_id = Some(get_registry_msg.registry_new_id);
            }
        }

        true
    }

    pub fn on_s2c_msg(&mut self, msg: &WlRawMsg) -> bool {
        if let Some(registry_obj_id) = self.registry_obj_id {
            if let Some(global_msg) =
                reject_malformed!(WlRegistryGlobalEvent::try_from_msg(registry_obj_id, msg))
            {
                println!(
                    "got global: {}, version {}",
                    global_msg.interface, global_msg.version
                );
            }
        }

        true
    }
}
