use crate::extensions::{
    Capability, Context, Contribution, Request,
    host::{Module, no_settings},
};

pub(crate) const ID: &str = "podman";

pub(crate) static MODULE: Module = Module {
    summary: None,
    registration,
    decorate: None,
    id: ID,
    capabilities: &[Capability::Collect],
    execute,
    settings: no_settings,
};

fn execute(context: &Context<'_>, request: Request<'_>) -> anyhow::Result<Contribution> {
    let Request::Collect = request else {
        anyhow::bail!("unsupported extension request");
    };
    let (runtime, containers) = crate::containers::collect_podman(context)?;
    Ok(Contribution::Collect {
        runtime,
        containers,
    })
}

fn registration() -> crate::extensions::placement::Registration {
    use crate::extensions::placement::*;
    Registration {
        categories: vec![Category::new("containers", "Containers", 150)],
        ownership: vec![],
    }
}
