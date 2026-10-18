//! Native execution ports consumed by workspace orchestration.
use crate::native_resolver::{
    adapter_profile,
    producers::{clang, go, java, python, roslyn, typescript},
};
use crate::{NativeProduceRequest, NativeProduceResult, NativeProducerError};
use entmoot_contracts::{AdapterProfile, BindingFamily};
use std::path::{Path, PathBuf};

pub trait AdapterDiscovery: Send + Sync {
    fn adapter_path(&self, family: &str) -> Option<PathBuf>;
    fn available(&self, family: &str) -> bool;
    fn discover(&self, families: Option<&[String]>) -> AdapterProfile;
    fn discover_and_store(&self, families: Option<&[String]>) -> Result<AdapterProfile, String>;
}
pub trait NativeProducer: AdapterDiscovery {
    fn produce_family(
        &self,
        family: BindingFamily,
        request: NativeProduceRequest,
    ) -> Result<NativeProduceResult, NativeProducerError>;
}
pub struct HostNative<'a> {
    root: &'a Path,
}
impl<'a> HostNative<'a> {
    pub fn new(root: &'a Path) -> Self {
        Self { root }
    }
}
impl AdapterDiscovery for HostNative<'_> {
    fn adapter_path(&self, family: &str) -> Option<PathBuf> {
        adapter_profile::with_workspace_root(self.root, || match family {
            "python" => python::configured_binary(),
            "typescript" => typescript::configured_binary(),
            "go" => go::configured_binary(),
            "roslyn" => roslyn::configured_binary(),
            "java" => java::configured_binary(),
            "clang" => clang::configured_binary(),
            _ => None,
        })
    }

    fn available(&self, family: &str) -> bool {
        adapter_profile::with_workspace_root(self.root, || match family {
            "python" => python::configured_binary().is_some(),
            "oxc" => cfg!(feature = "resolved-oxc"),
            "typescript" => typescript::configured_binary().is_some(),
            "rust" => cfg!(feature = "resolved-rust-analyzer"),
            "go" => go::configured_binary().is_some(),
            "roslyn" => roslyn::configured_binary().is_some(),
            "java" => java::configured_binary().is_some(),
            "clang" => clang::configured_binary().is_some(),
            _ => false,
        })
    }

    fn discover(&self, families: Option<&[String]>) -> AdapterProfile {
        adapter_profile::discover(self.root, families)
    }
    fn discover_and_store(&self, families: Option<&[String]>) -> Result<AdapterProfile, String> {
        adapter_profile::discover_and_store(self.root, families)
    }
}
impl NativeProducer for HostNative<'_> {
    fn produce_family(
        &self,
        family: BindingFamily,
        request: NativeProduceRequest,
    ) -> Result<NativeProduceResult, NativeProducerError> {
        adapter_profile::with_workspace_root(self.root, || {
            crate::native_resolver::pipeline::produce_family(family, request)
        })
    }
}
