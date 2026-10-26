use std::sync::Arc;

use crate::{
    environments::{opencode::Observer, state_stream::Store},
    store::opencode::{PromptedSession, Session, SessionPrompt},
    store::rules::DeliveryMode,
};

pub(in crate::service) struct BackgroundPrompt<'a> {
    pub input: &'a SessionPrompt,
    pub instructions: &'a str,
    pub delivery: DeliveryMode,
}

pub(in crate::service) async fn create_session(
    observer: &Observer,
    store: &Arc<Store>,
    session: &Session,
    prompt: BackgroundPrompt<'_>,
    instance: &str,
    origin: Option<&str>,
) -> Result<PromptedSession, String> {
    let adapter = store.clone();
    let directory = session.directory.clone();
    let name = instance.to_owned();
    let origin = origin.map(str::to_owned);
    let request = tokio::task::spawn_blocking(move || {
        adapter.begin_creation(&directory, &name, origin.as_deref())
    })
    .await
    .map_err(|error| error.to_string())??;
    let identity =
        (!session.id.is_empty()).then_some((session.id.as_str(), session.title.as_str()));
    let mut receipt = match observer
        .create_background_session(&session.directory, &session.server, prompt.input, identity)
        .await
    {
        Ok(receipt) => receipt,
        Err(error) => {
            let adapter = store.clone();
            tokio::task::spawn_blocking(move || adapter.cancel_creation(&request))
                .await
                .map_err(|error| error.to_string())??;
            return Err(error);
        }
    };
    let adapter = store.clone();
    let name = instance.to_owned();
    let created = receipt.clone();
    let binding =
        tokio::task::spawn_blocking(move || adapter.finish_creation(&request, &name, &created))
            .await
            .map_err(|error| error.to_string())?;
    if let Err(error) = binding {
        receipt.error = Some(format!(
            "Session identity persistence failed: {error}; prompt was not submitted"
        ));
        return Ok(receipt);
    }
    let Some(id) = &receipt.session_id else {
        return Ok(receipt);
    };
    let created = Session {
        id: id.clone(),
        ..session.clone()
    };
    Ok(observer
        .submit_background_prompt(
            &created,
            prompt.input,
            prompt.instructions,
            prompt.delivery,
            false,
        )
        .await)
}
