use std::env;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};

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
        "macos" => {
            set_mac(path)?;
        }
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
        .arg(format!("{command} file://{path}"))
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

fn set_mac(path: &str) -> Result<()> {
    let applescript = format!(
        r#"tell application "System Events" to set picture of every desktop to "{path}""#
    );

    Command::new("osascript")
        .arg("-e")
        .arg(&applescript)
        .output()
        .context("Failed to set wallpaper using AppleScript")?;

    Ok(())
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
