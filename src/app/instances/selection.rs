use crate::{
    app::{opencode::Target, rows::Row},
    store::opencode::Pane,
};

pub(super) fn pane_replacement(
    previous: &[Row],
    rows: &[Row],
    highlighted: Option<&str>,
) -> Option<String> {
    let highlighted = highlighted?;
    if rows.iter().any(|row| row.id == highlighted) {
        return None;
    }
    let pane = row_pane(previous.iter().find(|row| row.id == highlighted)?)?;
    rows.iter()
        .find(|row| {
            row_pane(row).is_some_and(|other| other.session == pane.session && other.id == pane.id)
        })
        .map(|row| row.id.clone())
}

fn row_pane(row: &Row) -> Option<&Pane> {
    match row.opencode.as_ref()? {
        Target::Client { pane }
        | Target::Session {
            pane: Some(pane), ..
        } => Some(pane),
        _ => None,
    }
}
