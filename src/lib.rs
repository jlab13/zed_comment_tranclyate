use std::fs;

use zed::settings::LspSettings;
use zed_extension_api as zed;

/// Назва виконуваного файлу сервера в PATH, в архіві релізу й у робочому каталозі.
const SERVER_BINARY: &str = "comment-translate-lsp";
/// Репозиторій GitHub, з релізів якого завантажується сервер.
const REPOSITORY: &str = "jlab13/zed_comment_translate";
/// Версія сервера дорівнює версії розширення, тому вони завжди сумісні.
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

struct CommentTranslate {
    /// Шлях до завантаженого сервера відносно робочого каталогу розширення.
    cached_binary_path: Option<String>,
}

impl CommentTranslate {
    /// Повертає шлях до сервера потрібної версії й завантажує його, якщо його ще немає.
    fn downloaded_binary_path(&mut self, id: &zed::LanguageServerId) -> zed::Result<String> {
        if let Some(path) = &self.cached_binary_path {
            if is_file(path) {
                return Ok(path.clone());
            }
        }

        let dir = version_dir(SERVER_VERSION);
        let binary_path = format!("{dir}/{SERVER_BINARY}");
        if !is_file(&binary_path) {
            zed::set_language_server_installation_status(
                id,
                &zed::LanguageServerInstallationStatus::Downloading,
            );
            if let Err(error) = download_server(&dir, &binary_path) {
                // Неповне встановлення не має вважатися готовим під час наступного запуску.
                fs::remove_dir_all(&dir).ok();
                zed::set_language_server_installation_status(
                    id,
                    &zed::LanguageServerInstallationStatus::Failed(error.clone()),
                );
                return Err(error);
            }
            remove_old_versions(&dir);
        }

        zed::set_language_server_installation_status(
            id,
            &zed::LanguageServerInstallationStatus::None,
        );
        self.cached_binary_path = Some(binary_path.clone());
        Ok(binary_path)
    }
}

impl zed::Extension for CommentTranslate {
    fn new() -> Self {
        Self {
            cached_binary_path: None,
        }
    }

    fn language_server_command(
        &mut self,
        id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> zed::Result<zed::Command> {
        // Явний шлях із налаштувань і сервер із PATH мають перевагу над завантаженим.
        let settings = LspSettings::for_worktree(id.as_ref(), worktree)?;
        let command = match settings
            .binary
            .and_then(|binary| binary.path)
            .or_else(|| worktree.which(SERVER_BINARY))
        {
            Some(path) => path,
            None => self.downloaded_binary_path(id)?,
        };
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

/// Повертає ціль Rust, для якої реліз містить готовий сервер.
fn release_target(os: zed::Os, arch: zed::Architecture) -> zed::Result<&'static str> {
    match (os, arch) {
        (zed::Os::Mac, zed::Architecture::Aarch64) => Ok("aarch64-apple-darwin"),
        (zed::Os::Mac, zed::Architecture::X8664) => Ok("x86_64-apple-darwin"),
        (zed::Os::Linux, zed::Architecture::X8664) => Ok("x86_64-unknown-linux-musl"),
        (zed::Os::Linux, zed::Architecture::Aarch64) => Ok("aarch64-unknown-linux-musl"),
        _ => Err(format!(
            "No prebuilt {SERVER_BINARY} is available for this platform. Build it from source \
             and set lsp.comment-translate.binary.path in your Zed settings."
        )),
    }
}

/// Назва архіву сервера для цілі в релізі GitHub.
fn asset_name(target: &str) -> String {
    format!("{SERVER_BINARY}-{target}.tar.gz")
}

/// Каталог сервера певної версії в робочому каталозі розширення.
fn version_dir(version: &str) -> String {
    format!("{SERVER_BINARY}-{version}")
}

/// Чи є запис робочого каталогу сервером іншої версії, ніж поточна.
fn is_stale_version_dir(name: &str, current_dir: &str) -> bool {
    name != current_dir && name.starts_with(&version_dir(""))
}

fn is_file(path: &str) -> bool {
    fs::metadata(path).is_ok_and(|metadata| metadata.is_file())
}

/// Завантажує архів сервера з релізу `v{SERVER_VERSION}` і розпаковує його в `dir`.
fn download_server(dir: &str, binary_path: &str) -> zed::Result<()> {
    let (os, arch) = zed::current_platform();
    let target = release_target(os, arch)?;
    let tag = format!("v{SERVER_VERSION}");
    let release = zed::github_release_by_tag_name(REPOSITORY, &tag)
        .map_err(|error| format!("Failed to find release {tag} of {REPOSITORY}: {error}"))?;
    let name = asset_name(target);
    let asset = release
        .assets
        .iter()
        .find(|asset| asset.name == name)
        .ok_or_else(|| format!("Release {tag} of {REPOSITORY} has no asset {name}"))?;

    // Прибирає залишки перерваного завантаження тієї ж версії.
    fs::remove_dir_all(dir).ok();
    zed::download_file(&asset.download_url, dir, zed::DownloadedFileType::GzipTar)
        .map_err(|error| format!("Failed to download {name}: {error}"))?;
    if !is_file(binary_path) {
        return Err(format!("{name} does not contain {SERVER_BINARY}"));
    }
    zed::make_file_executable(binary_path)
        .map_err(|error| format!("Failed to make {binary_path} executable: {error}"))
}

/// Видаляє сервери попередніх версій; помилки не заважають запуску поточної.
fn remove_old_versions(current_dir: &str) {
    let Ok(entries) = fs::read_dir(".") else {
        return;
    };
    for entry in entries.flatten() {
        if entry
            .file_name()
            .to_str()
            .is_some_and(|name| is_stale_version_dir(name, current_dir))
        {
            fs::remove_dir_all(entry.path()).ok();
        }
    }
}

zed::register_extension!(CommentTranslate);

#[cfg(test)]
mod tests {
    use super::*;

    /// Значення `version = "..."` з першого розділу TOML-файлу.
    fn manifest_version(manifest: &str) -> &str {
        manifest
            .lines()
            .find_map(|line| line.strip_prefix("version = \""))
            .and_then(|rest| rest.strip_suffix('"'))
            .expect("manifest has a version")
    }

    #[test]
    fn extension_and_server_versions_match_release_tag() {
        assert_eq!(
            manifest_version(include_str!("../extension.toml")),
            SERVER_VERSION
        );
        assert_eq!(
            manifest_version(include_str!("../server/Cargo.toml")),
            SERVER_VERSION
        );
    }

    #[test]
    fn supported_platforms_map_to_release_assets() {
        let cases = [
            (
                zed::Os::Mac,
                zed::Architecture::Aarch64,
                "aarch64-apple-darwin",
            ),
            (
                zed::Os::Mac,
                zed::Architecture::X8664,
                "x86_64-apple-darwin",
            ),
            (
                zed::Os::Linux,
                zed::Architecture::X8664,
                "x86_64-unknown-linux-musl",
            ),
            (
                zed::Os::Linux,
                zed::Architecture::Aarch64,
                "aarch64-unknown-linux-musl",
            ),
        ];
        for (os, arch, target) in cases {
            assert_eq!(release_target(os, arch), Ok(target));
        }
        assert_eq!(
            asset_name("x86_64-unknown-linux-musl"),
            "comment-translate-lsp-x86_64-unknown-linux-musl.tar.gz"
        );
    }

    #[test]
    fn unsupported_platforms_point_to_binary_path_setting() {
        let cases = [
            (zed::Os::Windows, zed::Architecture::X8664),
            (zed::Os::Windows, zed::Architecture::Aarch64),
            (zed::Os::Linux, zed::Architecture::X86),
        ];
        for (os, arch) in cases {
            let error = release_target(os, arch).unwrap_err();
            assert!(error.contains("lsp.comment-translate.binary.path"));
        }
    }

    #[test]
    fn only_other_server_versions_are_stale() {
        let current = version_dir("0.2.0");
        assert_eq!(current, "comment-translate-lsp-0.2.0");
        assert!(is_stale_version_dir(
            "comment-translate-lsp-0.1.0",
            &current
        ));
        assert!(!is_stale_version_dir(&current, &current));
        assert!(!is_stale_version_dir("other-data", &current));
    }
}
