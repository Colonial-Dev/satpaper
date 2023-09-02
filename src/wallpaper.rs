use std::path::Path;
use std::env;

use anyhow::{Result, Context};
use tokio::process::Command;

pub async fn set(path: impl AsRef<Path>) -> Result<()> {
    let path = path
        .as_ref()
        .to_str()
        .context("Failed to convert wallpaper path to a UTF-8 string")?;

    let desktop = env::var("XDG_CURRENT_DESKTOP")
        .context("Failed to get XDG_CURRENT_DESKTOP environment variable")?;
    
    log::debug!("XDG_CURRENT_DESKTOP is {desktop}.");
    log::debug!("Setting wallpaper to image at path {path}.");

    match desktop.as_str() {
        "GNOME" => set_gnome(path).await?,
        "KDE" => set_kde(path).await?,
        _ => panic!("Desktop {desktop} is not supported.")
    }

    Ok(())
}

async fn set_gnome(path: &str) -> Result<()> {
    let color_scheme = Command::new("gsettings")
        .args([
            "get",
            "org.gnome.desktop.interface",
            "color-scheme"
        ])
        .output()
        .await
        .context("Failed to get preferred color scheme from GSettings")?;

    let uri = match String::from_utf8(color_scheme.stdout)?.trim() {
        "'prefer-dark'" => "picture-uri-dark",
        _ => "picture-uri"
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

async fn set_kde(path: &str) -> Result<()> {
    // From https://superuser.com/questions/488232
    Command::new("qdbus")
        .args([
            "org.kde.plasmashell",
            "/PlasmaShell",
            "org.kde.PlasmaShell.evaluateScript",
            &format!(r#"'
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
        '"#)
        ])
        .output()
        .await
        .context("Failed to set wallpaper with qdbus")?;

    Ok(())
}