use crate::store::opencode::{
    Session,
    conversation::{LatestTurn, Message, latest_turn as select_latest_turn, transcript},
};

const QUESTION_MESSAGE_LIMIT: usize = 100;
const QUESTION_PAGE_LIMIT: usize = 10;

pub(crate) async fn load(session: &Session) -> Result<String, String> {
    if !super::valid_id(&session.id) {
        return Err("Invalid OpenCode conversation ID".into());
    }
    let mut path = reqwest::Url::parse("http://localhost/").map_err(|error| error.to_string())?;
    path.set_path(&format!("/session/{}/message", session.id));
    path.query_pairs_mut()
        .append_pair("directory", &session.directory);
    // Omitting limit asks OpenCode for every message; HTTP transport enforces the size bound.
    let target = format!("{}?{}", path.path(), path.query().unwrap_or_default());
    let messages = super::transport::get_optional::<Vec<Message>>(
        &super::transport::client()?,
        &session.server,
        &target,
    )
    .await?
    .ok_or("This conversation was deleted. Close this dialog to refresh the list.")?;
    Ok(transcript(messages))
}

pub(super) async fn latest_turn(
    client: &reqwest::Client,
    session: &Session,
) -> Result<Option<LatestTurn>, String> {
    if !super::valid_id(&session.id) {
        return Err("Invalid OpenCode conversation ID".into());
    }
    let mut before: Option<String> = None;
    for _ in 0..QUESTION_PAGE_LIMIT {
        let mut path =
            reqwest::Url::parse("http://localhost/").map_err(|error| error.to_string())?;
        path.set_path(&format!("/session/{}/message", session.id));
        {
            let mut query = path.query_pairs_mut();
            query
                .append_pair("directory", &session.directory)
                .append_pair("limit", &QUESTION_MESSAGE_LIMIT.to_string());
            if let Some(before) = &before {
                query.append_pair("before", before);
            }
        }
        let target = format!("{}?{}", path.path(), path.query().unwrap_or_default());
        let messages =
            super::transport::get_optional::<Vec<Message>>(client, &session.server, &target)
                .await?
                .unwrap_or_default();
        let next = messages.first().map(|message| message.info.id.clone());
        let full = messages.len() == QUESTION_MESSAGE_LIMIT;
        if let Some(turn) = select_latest_turn(messages) {
            return Ok(Some(turn));
        }
        if !full || next.is_none() || next == before {
            return Ok(None);
        }
        before = next;
    }
    Ok(None)
}
