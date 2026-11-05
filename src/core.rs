mod app_bundle;
mod bin;
mod command;

use crate::extensions::{Context, Contribution, Descriptor, Extension, Request, host::Module};

pub enum Core {
    AppBundle,
    Bin,
    Command,
}
impl Core {
    fn module(&self) -> &'static Module {
        match self {
            Self::AppBundle => &app_bundle::MODULE,
            Self::Bin => &bin::MODULE,
            Self::Command => &command::MODULE,
        }
    }
}
impl Extension for Core {
    fn descriptor(&self) -> Descriptor {
        let module = self.module();
        Descriptor {
            id: module.id.into(),
            capabilities: vec![module.capability],
            enabled: true,
            availability: "available",
        }
    }
    fn execute(&self, context: &Context<'_>, request: Request<'_>) -> anyhow::Result<Contribution> {
        (self.module().execute)(context, request)
    }
}
pub(crate) fn classifiers() -> [&'static dyn Extension; 2] {
    [&Core::AppBundle, &Core::Bin]
}
pub(crate) fn registration() -> crate::extensions::placement::Registration {
    use crate::extensions::placement::{Category, Ownership, Registration};
    Registration {
        categories: vec![
            Category::new("applications", "Applications", 100),
            Category::new("system", "System", 400),
            Category::new("unclassified", "Unclassified", 500),
        ],
        ownership: vec![
            Ownership::new("app", "applications", 100),
            Ownership::new("bin", "unclassified", 400),
        ],
    }
}
