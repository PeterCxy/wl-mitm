use std::{
    any::{Any, TypeId},
    collections::HashMap,
    hash::Hash,
};

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
    /// Objects that have been destroyed by the client, but not yet ACK'd by the server
    /// Objects in this state may still receive events from the server.
    objects_half_destroyed: HashMap<u32, WlObjectType>,
    object_extensions: HashMap<u32, HashMap<TypeId, Box<dyn Any + Send>>>,
    /// u32 "name"s of globals mapped to their object types
    global_names: HashMap<u32, WlObjectType>,
}

impl WlObjects {
    pub fn new() -> WlObjects {
        let mut objects = HashMap::new();
        objects.insert(WL_DISPLAY_OBJECT_ID, WL_DISPLAY);

        WlObjects {
            objects,
            objects_half_destroyed: HashMap::new(),
            object_extensions: HashMap::new(),
            global_names: Default::default(),
        }
    }

    pub fn record_object(&mut self, obj_type: WlObjectType, id: u32) {
        self.objects.insert(id, obj_type);
        self.object_extensions.remove(&id);
    }

    /// Returns [Some] if we have a record of that object ID. However,
    /// that object could have been destroyed by the client but not yet ACK'd
    /// by the server -- in that case, use [Self::is_half_destroyed]!
    pub fn lookup_object(&self, id: u32) -> Option<WlObjectType> {
        self.objects
            .get(&id)
            .or_else(|| self.objects_half_destroyed.get(&id))
            .cloned()
    }

    pub fn is_half_destroyed(&self, id: u32) -> bool {
        self.objects_half_destroyed.contains_key(&id)
    }

    pub fn remove_object(&mut self, id: u32, from_client: bool) {
        let is_client_object = id <= 0xFEFFFFFF;

        if from_client && is_client_object {
            // If a client destroys an object _it has created_, we don't remove it entirely;
            // we record it in another map and wait for the server to finally destroy it.
            // This is because the server may still send events before it ACK's the destruction request.
            let Some(old_entry) = self.objects.remove(&id) else {
                return;
            };
            self.objects_half_destroyed.insert(id, old_entry);
            self.object_extensions.remove(&id);
        } else {
            self.objects.remove(&id);
            self.objects_half_destroyed.remove(&id);
            self.object_extensions.remove(&id);
        }
    }

    pub fn put_object_extension<T: Any + Send>(&mut self, id: u32, extension: T) {
        if self.lookup_object(id).is_none() {
            // This should not happen but let's ignore extensions on non-existent objects
            return;
        }

        self.object_extensions
            .entry(id)
            .or_default()
            .insert(extension.type_id(), Box::new(extension));
    }

    pub fn get_object_extension<T: Any + Send>(&self, id: u32) -> Option<&T> {
        self.object_extensions
            .get(&id)?
            .get(&TypeId::of::<T>())?
            .downcast_ref()
    }

    pub fn get_object_extension_mut<T: Any + Send>(&mut self, id: u32) -> Option<&mut T> {
        self.object_extensions
            .get_mut(&id)?
            .get_mut(&TypeId::of::<T>())?
            .downcast_mut()
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
