use std::collections::BTreeMap;
use std::collections::btree_map::Entry;

use anyhow::{Result, bail};
use ekko_event::EventHandlerRegistration;

use crate::traits::ExtensionHost;
use crate::{
    CommandSpec, KeybindingSpec, ModeSpec, OverlaySpec, SessionGrouperSpec, SpinnerSpec,
    SurfaceSpec, ThemeSpec,
};

#[derive(Default)]
pub(crate) struct RegistryHost {
    pub(crate) commands: BTreeMap<String, CommandSpec>,
    /// Alias -> canonical command name.
    pub(crate) command_aliases: BTreeMap<String, String>,
    pub(crate) keybindings: Vec<KeybindingSpec>,
    pub(crate) modes: BTreeMap<String, ModeSpec>,
    pub(crate) surfaces: BTreeMap<String, SurfaceSpec>,
    pub(crate) overlays: BTreeMap<String, OverlaySpec>,
    pub(crate) themes: BTreeMap<String, ThemeSpec>,
    pub(crate) spinners: BTreeMap<String, SpinnerSpec>,
    pub(crate) session_grouper: Option<SessionGrouperSpec>,
    pub(crate) event_handlers: Vec<EventHandlerRegistration>,
}

impl RegistryHost {
    fn claim_command_name(&mut self, name: &str) -> Result<()> {
        if self.commands.contains_key(name) || self.command_aliases.contains_key(name) {
            bail!("command '{name}' is already registered");
        }
        Ok(())
    }
}

impl ExtensionHost for RegistryHost {
    fn register_command(&mut self, command: CommandSpec) -> Result<()> {
        self.claim_command_name(&command.name)?;
        for alias in &command.aliases {
            self.claim_command_name(alias)?;
        }
        for alias in &command.aliases {
            self.command_aliases
                .insert(alias.clone(), command.name.clone());
        }
        self.commands.insert(command.name.clone(), command);
        Ok(())
    }

    fn register_keybinding(&mut self, binding: KeybindingSpec) -> Result<()> {
        self.keybindings.push(binding);
        Ok(())
    }

    fn register_mode(&mut self, mode: ModeSpec) -> Result<()> {
        match self.modes.entry(mode.name.clone()) {
            Entry::Vacant(v) => {
                v.insert(mode);
                Ok(())
            }
            Entry::Occupied(o) => bail!("mode '{}' is already registered", o.key()),
        }
    }

    fn register_surface(&mut self, surface: SurfaceSpec) -> Result<()> {
        match self.surfaces.entry(surface.name.clone()) {
            Entry::Vacant(v) => {
                v.insert(surface);
                Ok(())
            }
            Entry::Occupied(o) => bail!("surface '{}' is already registered", o.key()),
        }
    }

    fn register_overlay(&mut self, overlay: OverlaySpec) -> Result<()> {
        match self.overlays.entry(overlay.name.clone()) {
            Entry::Vacant(v) => {
                v.insert(overlay);
                Ok(())
            }
            Entry::Occupied(o) => bail!("overlay '{}' is already registered", o.key()),
        }
    }

    fn register_theme(&mut self, theme: ThemeSpec) -> Result<()> {
        match self.themes.entry(theme.name.clone()) {
            Entry::Vacant(v) => {
                v.insert(theme);
                Ok(())
            }
            Entry::Occupied(o) => bail!("theme '{}' is already registered", o.key()),
        }
    }

    fn register_spinner(&mut self, spinner: SpinnerSpec) -> Result<()> {
        match self.spinners.entry(spinner.name.clone()) {
            Entry::Vacant(v) => {
                v.insert(spinner);
                Ok(())
            }
            Entry::Occupied(o) => bail!("spinner '{}' is already registered", o.key()),
        }
    }

    fn register_session_grouper(&mut self, grouper: SessionGrouperSpec) -> Result<()> {
        if let Some(existing) = &self.session_grouper {
            bail!(
                "session grouper '{}' is already registered (attempted '{}')",
                existing.name,
                grouper.name
            );
        }
        self.session_grouper = Some(grouper);
        Ok(())
    }

    fn subscribe(&mut self, handler: EventHandlerRegistration) -> Result<()> {
        self.event_handlers.push(handler);
        Ok(())
    }
}
