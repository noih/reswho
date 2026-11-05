use super::App;

pub(super) fn save(app: &mut App) {
    let Some(popup) = app.settings.as_mut() else {
        app.priority_error = Some("No settings context; cannot save".into());
        return;
    };
    match popup.context.save_extensions(
        app.priority_draft.as_deref(),
        app.ownership_draft.as_deref(),
        &app.builtin_draft,
    ) {
        Ok(applied) => {
            popup.applied = applied;
            app.priority_draft = Some(popup.context.baseline.source_priority.clone());
            app.ownership_draft = Some(popup.context.baseline.ownership_priority.clone());
            app.builtin_draft.clear();
            app.priority_error = None;
        }
        Err(error) => app.priority_error = Some(error.to_string()),
    }
}
