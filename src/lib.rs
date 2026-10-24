use zed::settings::LspSettings;
use zed_extension_api as zed;

struct CommentTranslate;

impl zed::Extension for CommentTranslate {
    fn new() -> Self {
        Self
    }

    fn language_server_command(
        &mut self,
        id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> zed::Result<zed::Command> {
        let settings = LspSettings::for_worktree(id.as_ref(), worktree)?;
        let configured_path = settings
            .binary
            .as_ref()
            .and_then(|binary| binary.path.clone());
        let command = configured_path
            .or_else(|| worktree.which("comment-translate-lsp"))
            .ok_or("Вкажіть lsp.comment-translate.binary.path або додайте comment-translate-lsp до PATH")?;
        Ok(zed::Command {
            command,
            args: vec![],
            env: vec![],
        })
    }

    fn language_server_initialization_options(
        &mut self,
        id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> zed::Result<Option<serde_json::Value>> {
        Ok(LspSettings::for_worktree(id.as_ref(), worktree)?.initialization_options)
    }
}

zed::register_extension!(CommentTranslate);
