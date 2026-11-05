use crate::extensions::{
    Capability, Context, Contribution, Request,
    host::{Module, no_settings},
};

pub(crate) const ID: &str = "docker";

pub(crate) static MODULE: Module = Module {
    registration,
    decorate: None,
    id: ID,
    capability: Capability::Collect,
    execute,
    settings: no_settings,
};

fn execute(context: &Context<'_>, request: Request<'_>) -> anyhow::Result<Contribution> {
    let Request::Collect = request else {
        anyhow::bail!("unsupported extension request");
    };
    let (mut runtime, mut containers) = crate::containers::collect_docker(context)?;
    crate::containers::retain_host_metadata(
        !context.settings.is_disabled("vm"),
        &mut runtime,
        &mut containers,
    );
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
