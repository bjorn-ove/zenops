use smol_str::SmolStr;

/// Identity fields that aren't git-specific — they're loaded from `[user]`
/// and also folded into [`Config::system_inputs`](super::Config::system_inputs)
/// as `user.name` / `user.email` so `ExpandStr` templates can reference them.
#[derive(serde::Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(default, deny_unknown_fields)]
pub(super) struct StoredUserConfig {
    pub name: Option<SmolStr>,
    pub email: Option<SmolStr>,
}
