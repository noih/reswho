use super::{assignment, assignment::Assignment, bundles};
use crate::model::Process;
use std::{collections::HashMap, path::PathBuf};

pub struct Facts<'a> {
    pub(crate) processes: &'a [Process],
    pub(crate) by_pid: HashMap<u32, &'a Process>,
    cache: std::sync::Mutex<HashMap<&'static str, Box<dyn std::any::Any + Send>>>,
    pub(crate) direct: HashMap<u32, Assignment>,
    pub(crate) executables: HashMap<u32, PathBuf>,
    pub(crate) bundles: bundles::Metadata,
}

#[cfg(test)]
thread_local! {
    pub(crate) static FACTS_BUILT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

impl<'a> Facts<'a> {
    pub(crate) fn cached<T: Default + Send + 'static, R>(
        &self,
        key: &'static str,
        use_cache: impl FnOnce(&mut T) -> R,
    ) -> R {
        let mut cache = self.cache.lock().expect("round facts cache");
        let value = cache.entry(key).or_insert_with(|| Box::<T>::default());
        use_cache(value.downcast_mut::<T>().expect("cache type"))
    }
    pub fn new(processes: &'a [Process]) -> Self {
        #[cfg(test)]
        FACTS_BUILT.with(|count| count.set(count.get() + 1));
        let direct = processes
            .iter()
            .map(|p| {
                let bundle = p.executable.as_deref().and_then(bundles::app_bundle);
                (p.pid, assignment::direct(p, bundle.as_ref()))
            })
            .collect();
        let executables = processes
            .iter()
            .filter_map(|p| {
                let path = p.executable.clone().filter(|path| path.is_absolute())?;
                Some((p.pid, path.canonicalize().unwrap_or(path)))
            })
            .collect();
        let bundles = bundles::read_metadata(processes.iter().flat_map(|p| {
            let executable = p.executable.as_deref();
            [
                executable.and_then(bundles::app_bundle),
                executable
                    .and_then(bundles::main_bundle)
                    .map(std::path::Path::to_path_buf),
            ]
            .into_iter()
            .flatten()
        }));
        Self {
            processes,
            by_pid: processes.iter().map(|p| (p.pid, p)).collect(),
            cache: Default::default(),
            direct,
            executables,
            bundles,
        }
    }
}
