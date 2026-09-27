pub(crate) const DEFAULT_FADE_SECONDS: u64 = 20;
pub(crate) const MAX_FADE_SECONDS: u64 = 3600;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SoundChoice {
    pub id: String,
    pub label: String,
}
