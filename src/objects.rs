use std::collections::HashMap;

use crate::proto::WL_DISPLAY_OBJECT_ID;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum WlObjectType {
    WlDisplay,
    WlRegistry,
}

pub struct WlObjects {
    objects: HashMap<u32, WlObjectType>,
    global_names: HashMap<u32, String>,
}

impl WlObjects {
    pub fn new() -> WlObjects {
        let mut objects = HashMap::new();
        objects.insert(WL_DISPLAY_OBJECT_ID, WlObjectType::WlDisplay);

        WlObjects {
            objects,
            global_names: Default::default(),
        }
    }

    pub fn record_object(&mut self, obj_type: WlObjectType, id: u32) {
        self.objects.insert(id, obj_type);
    }

    pub fn lookup_object(&self, id: u32) -> Option<WlObjectType> {
        self.objects.get(&id).cloned()
    }

    pub fn record_global(&mut self, name: u32, interface: &str) {
        self.global_names.insert(name, interface.to_string());
    }

    pub fn lookup_global(&self, name: u32) -> Option<&str> {
        self.global_names.get(&name).map(|s| s.as_str())
    }
}
