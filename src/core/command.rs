use crate::extensions::{
    Capability, Context, Contribution, DetailProposal, ProcessIdentity, Request,
    host::{Module, extension_command, no_settings},
};

pub(crate) const ID: &str = "command";

pub(crate) static MODULE: Module = Module {
    summary: None,
    registration: crate::extensions::host::no_registration,
    decorate: None,
    id: ID,
    capabilities: &[Capability::Details],
    execute,
    settings: no_settings,
};

fn execute(_: &Context<'_>, request: Request<'_>) -> anyhow::Result<Contribution> {
    let Request::Details {
        process,
        pointer_width,
    } = request
    else {
        anyhow::bail!("unsupported extension request");
    };
    let identity = ProcessIdentity::observed(process)
        .ok_or_else(|| anyhow::anyhow!("process identity unavailable"))?;
    Ok(Contribution::Details {
        proposal: DetailProposal {
            identity,
            command: extension_command(process, pointer_width),
        },
    })
}
