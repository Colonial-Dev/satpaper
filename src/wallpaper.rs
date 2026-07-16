use std::env;
#[cfg(any(target_os = "macos", test))]
use std::fs;
#[cfg(target_os = "macos")]
use std::fs::{File, OpenOptions};
#[cfg(target_os = "macos")]
use std::io;
use std::path::Path;
#[cfg(any(target_os = "macos", test))]
use std::path::PathBuf;
use std::process::Command;
#[cfg(target_os = "macos")]
use std::sync::atomic::AtomicBool;
#[cfg(any(target_os = "macos", test))]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(target_os = "macos")]
use std::sync::Mutex;
#[cfg(target_os = "macos")]
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(target_os = "macos")]
use anyhow::bail;
use anyhow::{Context, Result};

#[cfg(any(target_os = "macos", test))]
const MACOS_COPY_PREFIX: &str = ".satpaper-macos-wallpaper-v1-";
#[cfg(any(target_os = "macos", test))]
const MACOS_STARTUP_SCAN_LIMIT: usize = 256;
#[cfg(any(target_os = "macos", test))]
const MACOS_STARTUP_REMOVE_LIMIT: usize = 32;
#[cfg(target_os = "macos")]
const MACOS_SET_WALLPAPER_SCRIPT: &str = r#"
on run argv
    if (count of argv) is not 1 then error "expected one wallpaper path"
    set wallpaperPath to item 1 of argv
    tell application "System Events"
        repeat with desktopItem in every desktop
            set picture of desktopItem to wallpaperPath
        end repeat
    end tell
end run
"#;

#[cfg(target_os = "macos")]
static MACOS_COPY_SEQUENCE: AtomicU64 = AtomicU64::new(0);
#[cfg(target_os = "macos")]
static MACOS_STARTUP_CLEANUP_DONE: AtomicBool = AtomicBool::new(false);
#[cfg(target_os = "macos")]
static MACOS_ACTIVE_WALLPAPER: Mutex<Option<PathBuf>> = Mutex::new(None);

pub fn set(path: impl AsRef<Path>, user_command: Option<&str>) -> Result<()> {
    let path = path
        .as_ref()
        .to_str()
        .context("Failed to convert wallpaper path to a UTF-8 string")?;

    let os = env::consts::OS;

    log::debug!("Setting wallpaper to image at path {path}.");

    match os {
        "linux" => {
            if let Some(command) = user_command {
                return set_userdefined(path, command);
            }

            let desktop = env::var("XDG_CURRENT_DESKTOP")
                .context("Failed to get XDG_CURRENT_DESKTOP environment variable")?;

            match desktop.as_str() {
                // https://github.com/Colonial-Dev/satpaper/issues/7
                // Ubuntu don't be special for no reason challenge (impossible)
                "GNOME" | "ubuntu:GNOME" => set_gnome(path)?,
                "KDE" => set_kde(path)?,
                _ => panic!("Desktop {desktop} is not supported."),
            }
        }
        "windows" => {
            set_windows(path)?;
        }
        #[cfg(target_os = "macos")]
        "macos" => set_mac(path)?,
        _ => panic!("Operating system not supported."),
    }

    Ok(())
}

fn set_userdefined(path: &str, command: &str) -> Result<()> {
    #[cfg(target_family = "windows")]
    const SH_NAME: &str = "cmd";
    #[cfg(target_family = "windows")]
    const SH_ARG: &str = "/C";
    #[cfg(target_family = "unix")]
    const SH_NAME: &str = "sh";
    #[cfg(target_family = "unix")]
    const SH_ARG: &str = "-c";

    Command::new(SH_NAME)
        .arg(SH_ARG)
        .arg(format!("{command} {path}"))
        .output()
        .context("Failed to update wallpaper with custom command")?;

    Ok(())
}

fn set_gnome(path: &str) -> Result<()> {
    let color_scheme = Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "color-scheme"])
        .output()
        .context("Failed to get preferred color scheme from GSettings")?;

    let uri = match String::from_utf8(color_scheme.stdout)?.trim() {
        "'prefer-dark'" => "picture-uri-dark",
        _ => "picture-uri",
    };

    Command::new("gsettings")
        .args([
            "set",
            "org.gnome.desktop.background",
            uri,
            &format!("file://{path}"),
        ])
        .output()
        .context("GSettings failed to update wallpaper")?;

    Ok(())
}

fn set_windows(path: &str) -> Result<()> {
    // From https://c-nergy.be/blog/?p=15291
    //! IMPORTANT - DO NOT CHANGE THE FORMATTING OF THE POWERSHELL SCRIPT as this will BREAK the script. [more info: https://github.com/PowerShell/PowerShell/issues/2337]
    let powershell_script = format!(
        r#"
$code = @'
using System.Runtime.InteropServices;
namespace Win32 {{
    public class Wallpaper {{
        [DllImport("user32.dll", CharSet=CharSet.Auto)]
        static extern int SystemParametersInfo (int uAction, int uParam, string lpvParam, int fuWinIni);

        public static void SetWallpaper(string thePath) {{
            SystemParametersInfo(20, 0, thePath, 3);
        }}
    }}
}}
'@

add-type $code

# Apply the Change on the system
[Win32.Wallpaper]::SetWallpaper("{path}")"#
    );

    Command::new("powershell")
        .args([
            "-ExecutionPolicy",
            "Bypass",
            "-NoProfile",
            "-Command",
            &powershell_script,
        ])
        .output()
        .context("PowerShell failed to update wallpaper")?;

    Ok(())
}

#[cfg(target_os = "macos")]
fn set_mac(path: &str) -> Result<()> {
    let versioned_path = create_macos_wallpaper_copy(Path::new(path))?;
    let output = match Command::new("osascript")
        .arg("-e")
        .arg(MACOS_SET_WALLPAPER_SCRIPT)
        .arg("--")
        .arg(&versioned_path)
        .output()
    {
        Ok(output) => output,
        Err(error) => {
            remove_macos_copy(&versioned_path, "unused");
            return Err(error).context("Failed to run osascript to set the wallpaper");
        }
    };

    if !output.status.success() {
        remove_macos_copy(&versioned_path, "unused");

        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();
        let stderr = if stderr.is_empty() {
            "<no stderr>"
        } else {
            stderr
        };

        bail!(
            "osascript failed to set the wallpaper (status {}): {stderr}",
            output.status
        );
    }

    let previous = {
        let mut active = MACOS_ACTIVE_WALLPAPER
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        active.replace(versioned_path.clone())
    };

    if let Some(previous) = previous {
        remove_macos_copy(&previous, "superseded");
    }

    // Cleanup waits for the first successful switch. At that point no desktop
    // can still reference a copy left by an earlier Satpaper process.
    if !MACOS_STARTUP_CLEANUP_DONE.swap(true, Ordering::AcqRel) {
        match cleanup_stale_macos_copies(&versioned_path) {
            Ok(removed) if removed > 0 => {
                log::debug!("Removed {removed} stale macOS wallpaper copies.");
            }
            Ok(_) => {}
            Err(error) => {
                log::warn!("Failed to clean up stale macOS wallpaper copies: {error}");
            }
        }
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn create_macos_wallpaper_copy(source: &Path) -> Result<PathBuf> {
    let source = fs::canonicalize(source).with_context(|| {
        format!(
            "Failed to canonicalize macOS wallpaper path {}",
            source.display()
        )
    })?;
    let mut source_file = File::open(&source)
        .with_context(|| format!("Failed to open wallpaper at {}", source.display()))?;

    for _ in 0..8 {
        let sequence = MACOS_COPY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let unix_nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("System clock is before the Unix epoch")?
            .as_nanos();
        let destination =
            macos_wallpaper_copy_path(&source, std::process::id(), unix_nanos, sequence);

        let mut destination_file = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&destination)
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "Failed to create macOS wallpaper copy at {}",
                        destination.display()
                    )
                });
            }
        };

        if let Err(error) = io::copy(&mut source_file, &mut destination_file) {
            remove_macos_copy(&destination, "unused");
            return Err(error).with_context(|| {
                format!(
                    "Failed to copy wallpaper to macOS cache-busting path {}",
                    destination.display()
                )
            });
        }

        return Ok(destination);
    }

    bail!("Failed to allocate a unique macOS wallpaper filename after 8 attempts")
}

#[cfg(any(target_os = "macos", test))]
fn macos_wallpaper_copy_path(
    source: &Path,
    process_id: u32,
    unix_nanos: u128,
    sequence: u64,
) -> PathBuf {
    let mut filename = format!("{MACOS_COPY_PREFIX}{process_id}-{unix_nanos}-{sequence}");

    if let Some(extension) = source.extension().and_then(|extension| extension.to_str()) {
        filename.push('.');
        filename.push_str(extension);
    }

    source.with_file_name(filename)
}

#[cfg(any(target_os = "macos", test))]
fn is_macos_wallpaper_copy(path: &Path) -> bool {
    let Some(filename) = path.file_name().and_then(|filename| filename.to_str()) else {
        return false;
    };
    let Some(generated) = filename.strip_prefix(MACOS_COPY_PREFIX) else {
        return false;
    };
    let version = generated
        .split_once('.')
        .map_or(generated, |(version, _)| version);
    let mut parts = version.split('-');

    let (Some(process_id), Some(unix_nanos), Some(sequence), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };

    process_id.parse::<u32>().is_ok()
        && unix_nanos.parse::<u128>().is_ok()
        && sequence.parse::<u64>().is_ok()
}

#[cfg(any(target_os = "macos", test))]
fn select_stale_macos_copies(
    paths: impl IntoIterator<Item = PathBuf>,
    current: &Path,
    limit: usize,
) -> Vec<PathBuf> {
    paths
        .into_iter()
        .filter(|path| path != current && is_macos_wallpaper_copy(path))
        .take(limit)
        .collect()
}

#[cfg(any(target_os = "macos", test))]
fn cleanup_stale_macos_copies(current: &Path) -> Result<usize> {
    let directory = current
        .parent()
        .context("macOS wallpaper copy has no parent directory")?;
    let entries = fs::read_dir(directory).with_context(|| {
        format!(
            "Failed to read macOS wallpaper directory {}",
            directory.display()
        )
    })?;
    let paths = entries
        .take(MACOS_STARTUP_SCAN_LIMIT)
        .filter_map(|entry| match entry {
            Ok(entry) => Some(entry.path()),
            Err(error) => {
                log::warn!("Failed to inspect a macOS wallpaper directory entry: {error}");
                None
            }
        });
    let stale = select_stale_macos_copies(paths, current, MACOS_STARTUP_REMOVE_LIMIT);
    let mut removed = 0;

    for path in stale {
        match fs::remove_file(&path) {
            Ok(()) => removed += 1,
            Err(error) => {
                log::warn!(
                    "Failed to remove stale macOS wallpaper copy {}: {error}",
                    path.display()
                );
            }
        }
    }

    Ok(removed)
}

#[cfg(target_os = "macos")]
fn remove_macos_copy(path: &Path, description: &str) {
    if !is_macos_wallpaper_copy(path) {
        log::warn!(
            "Refusing to remove unrecognized macOS wallpaper path {}",
            path.display()
        );
        return;
    }

    if let Err(error) = fs::remove_file(path) {
        log::warn!(
            "Failed to remove {description} macOS wallpaper copy {}: {error}",
            path.display()
        );
    }
}

fn set_kde(path: &str) -> Result<()> {
    // the path has to be absolute to be set in the script
    let path = std::fs::canonicalize(path)?;
    let path = path.to_str().context("Failed to canonicalize the path")?;

    // adapted from https://superuser.com/questions/488232
    Command::new("qdbus")
        .args([
            "org.kde.plasmashell",
            "/PlasmaShell",
            "org.kde.PlasmaShell.evaluateScript",
            &format!(
                r#"
                var allDesktops = desktops();
                for (i=0;i<allDesktops.length;i++) {{
                    d = allDesktops[i];
                    d.wallpaperPlugin = "org.kde.image";
                    d.currentConfigGroup = Array("Wallpaper",
                                                 "org.kde.image",
                                                 "General");
                    // reset the current wallpaper, otherwise it is not reloaded
                    d.writeConfig("Image", null);
                    d.writeConfig("Image", "file://{path}");
                }}
                "#
            ),
        ])
        .status()
        .context("Failed to set wallpaper with qdbus")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(name: &str) -> Self {
            let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = env::temp_dir().join(format!(
                "satpaper-wallpaper-test-{name}-{}-{sequence}",
                std::process::id()
            ));
            fs::create_dir(&path).expect("failed to create test directory");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).expect("failed to remove test directory");
        }
    }

    #[test]
    fn macos_copy_name_is_unique_sibling_with_matching_extension() {
        let source = Path::new("/tmp/wallpapers/satpaper_latest.png");

        let copy = macos_wallpaper_copy_path(source, 42, 1_234_567, 8);

        assert_eq!(
            copy,
            Path::new("/tmp/wallpapers/.satpaper-macos-wallpaper-v1-42-1234567-8.png")
        );
        assert!(is_macos_wallpaper_copy(&copy));
    }

    #[test]
    fn stale_copy_selection_honors_removal_limit() {
        let source = Path::new("/tmp/wallpapers/satpaper_latest.png");
        let current = macos_wallpaper_copy_path(source, 42, 100, 5);
        let paths = (0..5).map(|sequence| macos_wallpaper_copy_path(source, 42, 100, sequence));

        let selected = select_stale_macos_copies(paths, &current, 2);

        assert_eq!(selected.len(), 2);
    }

    #[test]
    fn startup_cleanup_removes_only_stale_generated_copies() {
        let directory = TestDirectory::new("cleanup");
        let source = directory.path().join("satpaper_latest.png");
        let current = macos_wallpaper_copy_path(&source, 42, 100, 3);
        let stale_one = macos_wallpaper_copy_path(&source, 42, 100, 1);
        let stale_two = macos_wallpaper_copy_path(&source, 42, 100, 2);
        let user_file = directory.path().join("holiday.png");
        let deceptive_file = directory
            .path()
            .join(".satpaper-macos-wallpaper-v1-user-copy.png");

        for path in [
            &current,
            &stale_one,
            &stale_two,
            &user_file,
            &deceptive_file,
        ] {
            fs::write(path, b"image").expect("failed to write test file");
        }

        let removed = cleanup_stale_macos_copies(&current).expect("cleanup failed");

        assert_eq!(removed, 2);
        assert!(current.exists());
        assert!(!stale_one.exists());
        assert!(!stale_two.exists());
        assert!(user_file.exists());
        assert!(deceptive_file.exists());
    }
}
