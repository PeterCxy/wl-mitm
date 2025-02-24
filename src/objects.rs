use std::collections::HashMap;

use crate::proto::WL_DISPLAY_OBJECT_ID;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum WlObjectType {
    WlDisplay,
    WlRegistry,
}

pub struct WlObjects {
    objects: HashMap<u32, WlObjectType>,
}

impl WlObjects {
    pub fn new() -> WlObjects {
        let mut objects = HashMap::new();
        objects.insert(WL_DISPLAY_OBJECT_ID, WlObjectType::WlDisplay);

        WlObjects { objects }
    }

    pub fn record_object(&mut self, obj_type: WlObjectType, id: u32) {
        self.objects.insert(id, obj_type);
    }

    pub fn lookup_object(&self, id: u32) -> Option<WlObjectType> {
        self.objects.get(&id).cloned()
    }
}
