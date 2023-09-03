<h1 align="center">Satpaper</h1>
<h3 align="center">Display near-real-time satellite imagery on your desktop.</h3>

<p align = "center">
<img src=".github/satpaper_latest.png" width = 768>
<br>
<i> (Click to see full-size version) </i>
</p>

Satpaper generates live wallpapers for your desktop, using near-real-time imagery from [RAMMB SLIDER](https://rammb-slider.cira.colostate.edu).

There are several satellites to choose from, each covering a different region of the world.
- GOES East (used in the sample image - covers most of North and South America)
- GOES West (Pacific Ocean and parts of the western US)
- Himawari (Oceania and East Asia)
- Meteosat 9 (Africa, Middle East, India, Central Asia)
- Meteosat 10 (Atlantic Ocean, Africa, Europe)

## Warning - Data Usage
Satpaper downloads satellite imagery at the highest available resolution and downscales it to fit your specifications. The exact download size varies depending on which satellite you are using and the image contents, but it's typically in the ballpark of twenty megabytes.

If you're on a metered and/or severely bandwidth-limited connection, twenty megabytes every ten to fifteen minutes can really add up. You have been warned!

## Installation
Environments with automatic support:
- GNOME
- KDE
- Windows (tested to work on 10/11)
- macOS (tested on Ventura)
    - Satpaper will ask for System Event permission when running for the first time - you will need to grant access then restart the program for it to work.

If your environment is not supported, you can use the `--wallpaper-command`/`SATPAPER_WALLPAPER_COMMAND` argument to specify a command to run whenever a new wallpaper is generated. PRs to add automatic support are also welcome!

Dependencies:
- The most recent stable [Rust toolchain](https://rustup.rs/).
- A C/C++ toolchain (such as `gcc`.)
- OpenSSL source (packaged as `libssl-dev` on Debian/Ubuntu and `openssl-devel` on Red Hat.)

Just use `cargo install`, and Satpaper will be compiled and added to your `PATH`.
```sh
cargo install --locked --git https://github.com/Colonial-Dev/satpaper --branch master
```

To automatically start Satpaper when you log in, you can use a `systemd` unit or equivalent.

```
[Unit]
Description=Run Satpaper on login.

# You should adjust these values as needed/preferred.
[Service]
Environment=SATPAPER_SATELLITE=goes-east
Environment=SATPAPER_RESOLUTION_X=2560
Environment=SATPAPER_RESOLUTION_Y=1440
Environment=SATPAPER_DISK_SIZE=94
Environment=SATPAPER_TARGET_PATH=/var/home/colonial/.local/share/backgrounds/

ExecStart=/var/home/colonial/.cargo/bin/satpaper
Restart=on-failure
RestartSec=1

[Install]
WantedBy=default.target
```

```sh
# (Write out or paste in your unit file)
nano $HOME/.config/systemd/user/satpaper.service
systemctl --user enable satpaper
systemctl --user start satpaper
```

## FAQ

### *Why are continents purple in night imagery?* / *Why does night imagery look kinda weird?*
This is a byproduct of the CIRA GeoColor processing algorithm used to generate full-color images from the raw satellite data. GeoColor uses infrared imaging for night-time imaging, which is then overlaid with false city lights and whitened clouds. The resulting image usually looks pretty good at a glance, but might begin to seem unnatural upon closer inspection.

Unfortunately, this is a necessary evil, as geostationary weather satellites don't capture enough visible spectrum light to generate a true-color night-time image.

### *I live at `$EXTREME_LATITUDE` - is there a way to get better imagery of my location?*
Not really. Geostationary orbits (required for the type of imaging we want) can only be achieved at a very specific altitude directly above the equator.
