use crate::{
    codec::WlRawMsg,
    objects::{WlObjectType, WlObjects},
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
    objects: WlObjects,
}

impl WlMitmState {
    pub fn new() -> WlMitmState {
        WlMitmState {
            objects: WlObjects::new(),
        }
    }

    pub fn on_c2s_msg(&mut self, msg: &WlRawMsg) -> bool {
        if let Some(get_registry_msg) =
            reject_malformed!(WlDisplayGetRegistry::try_from_msg(&self.objects, msg))
        {
            self.objects
                .record_object(WlObjectType::WlRegistry, get_registry_msg.registry_new_id);
        }

        true
    }

    pub fn on_s2c_msg(&mut self, msg: &WlRawMsg) -> bool {
        if let Some(global_msg) =
            reject_malformed!(WlRegistryGlobalEvent::try_from_msg(&self.objects, msg))
        {
            println!(
                "got global: {}, name {}, version {}",
                global_msg.interface, global_msg.name, global_msg.version
            );

            self.objects
                .record_global(global_msg.name, global_msg.interface);
        }

        true
    }
}
