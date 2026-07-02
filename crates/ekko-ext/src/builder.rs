use anyhow::{Context, Result};

use crate::host::RegistryHost;
use crate::runtime::AppRuntime;
use crate::traits::Extension;

/// Collects extensions and assembles them into an immutable [`AppRuntime`].
/// Client and server each build their own runtime with the same builder.
#[derive(Default)]
pub struct RuntimeBuilder {
    extensions: Vec<Box<dyn Extension>>,
    disabled: Vec<String>,
}

impl RuntimeBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Manifest ids to skip at build time (`[extensions] disabled` config).
    pub fn with_disabled(mut self, ids: &[String]) -> Self {
        self.disabled = ids.to_vec();
        self
    }

    pub fn register_extension<E>(self, extension: E) -> Self
    where
        E: Extension + 'static,
    {
        self.register_boxed_extension(Box::new(extension))
    }

    pub fn register_boxed_extension(mut self, extension: Box<dyn Extension>) -> Self {
        self.extensions.push(extension);
        self
    }

    /// Register a batch of boxed extensions (a builtins layer, a script
    /// loader's crop) in order.
    pub fn register_boxed_extensions(
        self,
        extensions: impl IntoIterator<Item = Box<dyn Extension>>,
    ) -> Self {
        extensions
            .into_iter()
            .fold(self, Self::register_boxed_extension)
    }

    pub fn build(self) -> Result<AppRuntime> {
        let mut host = RegistryHost::default();
        let mut manifests = Vec::new();

        for extension in &self.extensions {
            let manifest = extension.manifest();
            if self.disabled.contains(&manifest.id) {
                continue;
            }
            extension
                .register(&mut host)
                .with_context(|| format!("registering extension '{}'", manifest.id))?;
            manifests.push(manifest);
        }

        Ok(AppRuntime::from_registry(manifests, host))
    }
}
