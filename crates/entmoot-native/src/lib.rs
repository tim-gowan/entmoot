//! Explicit native providers and isolated process execution.
pub mod materialize;
pub mod native_resolver;
pub mod process;
pub mod producer;
pub use entmoot_evidence::*;
pub use native_resolver::{
    prepare_resolver_manifest, produce_clang_resolver, produce_go_resolver, produce_java_resolver,
    produce_oxc_resolver, produce_python_resolver, produce_roslyn_resolver, produce_rust_resolver,
    produce_typescript_resolver, publish_dogfood, DogfoodPublishRequest, DogfoodPublishResult,
    ExecutionPolicy, NativeProduceRequest, NativeProduceResult, NativeProducerError, NativeReceipt,
    ProducerBudget, PublicationManifest, PUBLICATION_SCHEMA, RA_AP_PIN,
};
pub use producer::AssertError;

pub mod execution;
pub mod services;
pub use services::{AdapterDiscovery, HostNative, NativeProducer};

pub use native_resolver::package_api_seek::PackageRootResolver;
