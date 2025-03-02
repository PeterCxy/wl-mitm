use std::{any::Any, collections::HashMap, hash::Hash};

use crate::proto::{WL_DISPLAY, WL_DISPLAY_OBJECT_ID};

/// A type ID to be implemented by _private structs_ acting as
/// discriminants for Wayland object types
/// Note: [Any] is required for the [PartialEq] impl to work!
/// Otherwise, when we cast to [Any], we're just going to use
/// the type id of [dyn WlObjectTypeId] instead!
pub trait WlObjectTypeId: Any + Send + Sync {
    fn interface(&self) -> &'static str;
}

/// A dyn, static reference of a [WlObjectTypeId]. This acts
/// as the public-facing discriminant value for Wayland objects.
///
/// We use this roundabount way to express types rather than using
/// an enum, because we don't want the [WlObjectType] to be sealed:
/// we should be able to add wl protocols anywhere anytime.
#[derive(Clone, Copy)]
pub struct WlObjectType(pub &'static dyn WlObjectTypeId);

impl WlObjectType {
    pub const fn new(id: &'static dyn WlObjectTypeId) -> WlObjectType {
        WlObjectType(id)
    }

    #[allow(dead_code)]
    pub fn interface(&self) -> &'static str {
        self.0.interface()
    }
}

impl PartialEq for WlObjectType {
    fn eq(&self, other: &Self) -> bool {
        // This _requires_ the [Any] supertrait in [WlObjectTypeid]
        self.0.type_id().eq(&other.0.type_id())
    }
}

impl Eq for WlObjectType {}

impl Hash for WlObjectType {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.interface().hash(state);
    }
}

pub struct WlObjects {
    objects: HashMap<u32, WlObjectType>,
    /// u32 "name"s of globals mapped to their object types
    global_names: HashMap<u32, WlObjectType>,
}

impl WlObjects {
    pub fn new() -> WlObjects {
        let mut objects = HashMap::new();
        objects.insert(WL_DISPLAY_OBJECT_ID, WL_DISPLAY);

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

    pub fn remove_object(&mut self, id: u32) {
        self.objects.remove(&id);
    }

    pub fn record_global(&mut self, name: u32, interface: WlObjectType) {
        self.global_names.insert(name, interface);
    }

    pub fn lookup_global(&self, name: u32) -> Option<WlObjectType> {
        self.global_names.get(&name).copied()
    }

    pub fn remove_global(&mut self, name: u32) {
        self.global_names.remove(&name);
    }
}
