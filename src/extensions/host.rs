pub(crate) mod assignment;
pub(crate) mod bundles;
pub mod facts;
pub(crate) mod files;
pub(crate) mod projects;
pub(crate) mod proposals;
pub(crate) mod runtime;

pub(crate) use crate::collect::{extension_command, sanitize_command};
pub use facts::Facts;

pub(crate) fn stable_ref(category: &str, key: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(format!("{category}\0{key}"));
    format!("group:{}", &format!("{digest:x}")[..16])
}

use crate::extensions::{Capability, Context, Contribution, Request};
use crate::settings::Definition;

pub type Decoration = fn(&Facts<'_>, &mut crate::extensions::tags::Tag);

pub(crate) struct Module {
    pub(crate) registration: fn() -> crate::extensions::placement::Registration,
    pub(crate) decorate: Option<Decoration>,
    pub(crate) id: &'static str,
    pub(crate) capability: Capability,
    pub(crate) execute: fn(&Context<'_>, Request<'_>) -> anyhow::Result<Contribution>,
    pub(crate) settings: fn() -> anyhow::Result<Vec<Definition>>,
}

pub(crate) fn no_settings() -> anyhow::Result<Vec<Definition>> {
    Ok(Vec::new())
}

pub(crate) fn no_registration() -> crate::extensions::placement::Registration {
    Default::default()
}
pub(crate) mod vm;
