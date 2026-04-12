#![forbid(unsafe_code)]

pub mod fs_backend;
pub mod index;
pub mod mounter;
pub mod publisher;
pub mod registry;
pub mod resource;

pub use fs_backend::*;
pub use index::{
    ResourceEntry, ResourceEvent, ResourceId, ResourceIndex, ResourceMetadata,
    ResourceMetadataPatch, ResourcePublishedEvent, ResourceUnpublishedEvent,
    ResourceUpdatedEvent,
};
pub use mounter::{
    DurableStreamMounter, LocalPathMounter, MountedResource, ResourceMounter,
    prepare_resources,
};
pub use publisher::ResourcePublisher;
pub use registry::{
    ResourceRegistry, ResourceWatcher, StreamResourceRegistry, Subscription,
};
pub use resource::{HostId, PublishedResourceRef, ResourceRef, ResourceSourceRef, StreamFsMode};
