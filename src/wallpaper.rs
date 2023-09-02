use anyhow::{Context, Result};
use std::env;
use std::path::Path;
use tokio::process::Command;

pub async fn set(path: impl AsRef<Path>, user_command: Option<&str>) -> Result<()> {
    let path = path
        .as_ref()
        .to_str()
        .context("Failed to convert wallpaper path to a UTF-8 string")?;

    let os = env::consts::OS;

    log::debug!("Setting wallpaper to image at path {path}.");

    match os {
        "linux" => {
            let desktop = env::var("XDG_CURRENT_DESKTOP")
                .context("Failed to get XDG_CURRENT_DESKTOP environment variable")?;

            match user_command {
                Some(command) => set_userdefined(path, command).await?,
                None => match desktop.as_str() {
                    "GNOME" => set_gnome(path).await?,
                    "KDE" => set_kde(path).await?,
                    _ => panic!("Desktop {desktop} is not supported."),
                },
            }
        }
        "windows" => {
            set_windows(path).await?;
        }
        _ => panic!("Operating system not supported."),
    }

    Ok(())
}

async fn set_userdefined(path: &str, command: &str) -> Result<()> {
    Command::new("sh")
        .args(["-c", &format!("{command} file://{path}")])
        .output()
        .await
        .context("failed to update wallpaper")?;

    Ok(())
}

async fn set_gnome(path: &str) -> Result<()> {
    let color_scheme = Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "color-scheme"])
        .output()
        .await
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
        .await
        .context("GSettings failed to update wallpaper")?;

    Ok(())
}

async fn set_windows(path: &str) -> Result<()> {
    // From https://c-nergy.be/blog/?p=15291
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
[Win32.Wallpaper]::SetWallpaper("{}")"#,
        path
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
        .await
        .context("PowerShell failed to update wallpaper")?;

    Ok(())
}

async fn set_kde(path: &str) -> Result<()> {
    // From https://superuser.com/questions/488232
    Command::new("qdbus")
        .args([
            "org.kde.plasmashell",
            "/PlasmaShell",
            "org.kde.PlasmaShell.evaluateScript",
            &format!(
                r#"'
                var allDesktops = desktops();
                print (allDesktops);
                for (i=0;i<allDesktops.length;i++) {{
                    d = allDesktops[i];
                    d.wallpaperPlugin = "org.kde.image";
                    d.currentConfigGroup = Array("Wallpaper",
                                                "org.kde.image",
                                                "General");
                    d.writeConfig("Image", "file://{path}")
                }}
        '"#
            ),
        ])
        .output()
        .await
        .context("Failed to set wallpaper with qdbus")?;

    Ok(())
}
