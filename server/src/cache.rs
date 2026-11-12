// Постійний кеш перекладів на диску: один файл на абзац, ключ — SHA-256.
// Записи живуть обмежений час, щоб з часом отримати кращий переклад від сервісу.

use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime},
};

/// Версія формату ключа: після змін у поділі на абзаци чи мітках старі записи не збігаються.
const FORMAT: &str = "v1";
/// Найбільший розмір кешу; при запуску найстаріші записи понад нього видаляються.
pub const MAX_BYTES: u64 = 50 * 1024 * 1024;

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug)]
pub struct DiskCache {
    root: PathBuf,
    ttl: Duration,
}

/// Каталог кешу: `$XDG_CACHE_HOME`, на macOS `~/Library/Caches`, інакше `~/.cache`.
pub fn default_dir(variable: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let xdg = variable("XDG_CACHE_HOME").filter(|dir| Path::new(dir).is_absolute());
    let base = match xdg {
        Some(dir) => PathBuf::from(dir),
        None => {
            let home = PathBuf::from(variable("HOME").filter(|home| !home.is_empty())?);
            if cfg!(target_os = "macos") {
                home.join("Library").join("Caches")
            } else {
                home.join(".cache")
            }
        }
    };
    Some(base.join("comment-translate"))
}

/// Ключ запису: провайдер, мови й текст абзацу, щоб зміна будь-чого давала новий переклад.
pub fn key(provider: &str, source: &str, target: &str, text: &str) -> String {
    let mut hasher = Sha256::new();
    for part in [FORMAT, provider, source, target, text] {
        // Довжина перед кожною частиною не дає різним наборам частин дати той самий рядок.
        hasher.update((part.len() as u64).to_le_bytes());
        hasher.update(part.as_bytes());
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

impl DiskCache {
    pub fn new(root: PathBuf, ttl: Duration) -> Self {
        Self { root, ttl }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn entries(&self) -> PathBuf {
        self.root.join(FORMAT)
    }

    fn path(&self, key: &str) -> PathBuf {
        // Два перші символи — підкаталог, щоб в одному каталозі не було десятків тисяч файлів.
        self.entries().join(&key[..2]).join(key)
    }

    fn expired(&self, path: &Path) -> bool {
        fs::metadata(path)
            .and_then(|metadata| metadata.modified())
            .map_or(true, |written| {
                SystemTime::now()
                    .duration_since(written)
                    .is_ok_and(|age| age > self.ttl)
            })
    }

    /// Переклад із кешу, якщо він є і ще не застарів; застарілий запис видаляється.
    pub fn get(&self, key: &str) -> Option<String> {
        let path = self.path(key);
        if self.expired(&path) {
            fs::remove_file(&path).ok();
            return None;
        }
        fs::read_to_string(&path).ok()
    }

    /// Зберігає переклад; помилки запису не заважають перекладу, тому ігноруються.
    pub fn put(&self, key: &str, translation: &str) {
        let path = self.path(key);
        let Some(directory) = path.parent() else {
            return;
        };
        if create_private_dir(directory).is_err() {
            return;
        }
        // Запис через тимчасовий файл і перейменування: інший сервер не прочитає половину запису.
        let temporary = directory.join(format!(
            ".{key}.{}.{}",
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let written = create_private_file(&temporary)
            .and_then(|mut file| file.write_all(translation.as_bytes()))
            .and_then(|()| fs::rename(&temporary, &path));
        if written.is_err() {
            fs::remove_file(&temporary).ok();
        }
    }

    /// Видаляє застарілі записи, а потім найстаріші, доки кеш не вміститься в `max_bytes`.
    pub fn prune(&self, max_bytes: u64) {
        let Ok(groups) = fs::read_dir(self.entries()) else {
            return;
        };
        let mut kept = Vec::new();
        for group in groups.flatten() {
            let Ok(files) = fs::read_dir(group.path()) else {
                continue;
            };
            for file in files.flatten() {
                let path = file.path();
                let Ok(metadata) = file.metadata() else {
                    continue;
                };
                if !metadata.is_file() {
                    continue;
                }
                if self.expired(&path) {
                    fs::remove_file(&path).ok();
                } else {
                    let written = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                    kept.push((written, metadata.len(), path));
                }
            }
        }
        let mut total: u64 = kept.iter().map(|(_, size, _)| size).sum();
        kept.sort_by_key(|(written, _, _)| *written);
        for (_, size, path) in kept {
            if total <= max_bytes {
                break;
            }
            if fs::remove_file(&path).is_ok() {
                total -= size;
            }
        }
    }

    /// Прибирає кеш в окремому потоці, щоб не затримувати запуск сервера.
    pub fn prune_in_background(&self) {
        let cache = self.clone();
        std::thread::spawn(move || cache.prune(MAX_BYTES));
    }
}

// Каталоги кешу доступні лише власнику: у них перекладений текст коментарів.
fn create_private_dir(path: &Path) -> std::io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)
}

fn create_private_file(path: &Path) -> std::io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Тимчасовий каталог тесту; видаляється разом зі значенням.
    pub struct TempDir(pub PathBuf);

    impl TempDir {
        pub fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "comment-translate-{name}-{}-{}",
                std::process::id(),
                TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            fs::remove_dir_all(&path).ok();
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).ok();
        }
    }

    const DAY: Duration = Duration::from_secs(24 * 60 * 60);

    fn age(cache: &DiskCache, key: &str, by: Duration) {
        let file = fs::File::options()
            .write(true)
            .open(cache.path(key))
            .unwrap();
        file.set_modified(SystemTime::now() - by).unwrap();
    }

    #[test]
    fn key_is_stable_and_depends_on_every_part() {
        let base = key("deepl", "auto", "uk", "Return the value.");
        assert_eq!(base.len(), 64);
        assert_eq!(base, key("deepl", "auto", "uk", "Return the value."));
        for other in [
            key("google", "auto", "uk", "Return the value."),
            key("deepl", "en", "uk", "Return the value."),
            key("deepl", "auto", "de", "Return the value."),
            key("deepl", "auto", "uk", "Return the value!"),
            // Межі між частинами входять у ключ.
            key("deepl", "autouk", "", "Return the value."),
        ] {
            assert_ne!(base, other);
        }
    }

    #[test]
    fn stores_and_reads_translations() {
        let dir = TempDir::new("roundtrip");
        let cache = DiskCache::new(dir.0.clone(), DAY);
        let entry = key("deepl", "auto", "uk", "Return the value 😀.");
        assert_eq!(cache.get(&entry), None);
        cache.put(&entry, "Повернути значення 😀.\nДругий рядок.");
        assert_eq!(
            cache.get(&entry).as_deref(),
            Some("Повернути значення 😀.\nДругий рядок.")
        );
        cache.put(&entry, "Новий переклад.");
        assert_eq!(cache.get(&entry).as_deref(), Some("Новий переклад."));
        // Тимчасових файлів не лишається.
        let group = cache.path(&entry).parent().unwrap().to_path_buf();
        assert_eq!(fs::read_dir(group).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn cache_is_private_to_the_owner() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new("private");
        let cache = DiskCache::new(dir.0.clone(), DAY);
        let entry = key("google", "auto", "uk", "Text");
        cache.put(&entry, "Текст");
        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir.0), 0o700);
        assert_eq!(mode(cache.path(&entry).parent().unwrap()), 0o700);
        assert_eq!(mode(&cache.path(&entry)), 0o600);
    }

    #[test]
    fn expired_translations_are_ignored_and_removed() {
        let dir = TempDir::new("expiry");
        let cache = DiskCache::new(dir.0.clone(), 14 * DAY);
        let entry = key("google", "auto", "uk", "Old");
        cache.put(&entry, "Старий");
        age(&cache, &entry, 13 * DAY);
        assert_eq!(cache.get(&entry).as_deref(), Some("Старий"));
        age(&cache, &entry, 15 * DAY);
        assert_eq!(cache.get(&entry), None);
        assert!(!cache.path(&entry).exists());
    }

    #[test]
    fn prune_removes_expired_then_oldest_entries() {
        let dir = TempDir::new("prune");
        let cache = DiskCache::new(dir.0.clone(), 14 * DAY);
        let keys: Vec<_> = (0..4)
            .map(|index| key("google", "auto", "uk", &index.to_string()))
            .collect();
        for (index, entry) in keys.iter().enumerate() {
            cache.put(entry, &"x".repeat(100));
            age(&cache, entry, Duration::from_secs(60 * (10 - index as u64)));
        }
        age(&cache, &keys[3], 20 * DAY);
        // Три свіжі записи по 100 байтів, ліміт 250: видаляється найстаріший з них.
        cache.prune(250);
        let present: HashMap<_, _> = keys
            .iter()
            .map(|entry| (entry.clone(), cache.path(entry).exists()))
            .collect();
        assert!(!present[&keys[0]], "найстаріший свіжий запис понад ліміт");
        assert!(present[&keys[1]] && present[&keys[2]]);
        assert!(!present[&keys[3]], "застарілий запис");
        cache.prune(MAX_BYTES);
        assert!(cache.path(&keys[1]).exists());
    }

    #[test]
    fn prune_without_a_cache_directory_does_nothing() {
        let dir = TempDir::new("missing");
        DiskCache::new(dir.0.join("absent"), DAY).prune(0);
        assert!(!dir.0.exists());
    }

    #[test]
    fn default_directory_follows_platform_conventions() {
        let lookup = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| value.to_string())
            }
        };
        assert_eq!(
            default_dir(lookup(&[("XDG_CACHE_HOME", "/xdg"), ("HOME", "/home/u")])),
            Some(PathBuf::from("/xdg/comment-translate"))
        );
        // Відносний XDG_CACHE_HOME не дійсний за специфікацією XDG.
        let home = default_dir(lookup(&[("XDG_CACHE_HOME", "rel"), ("HOME", "/home/u")]));
        let expected = if cfg!(target_os = "macos") {
            "/home/u/Library/Caches/comment-translate"
        } else {
            "/home/u/.cache/comment-translate"
        };
        assert_eq!(home, Some(PathBuf::from(expected)));
        assert_eq!(default_dir(lookup(&[])), None);
    }
}
