### September 3, 2023 / v0.5.0
- Added `--once` flag to make program usable in non-daemon context
- Switched to `mimalloc` for greater control over when memory is returned to the OS
- Moved away from `tokio`, refactoring code to be sync.
    - Faster compile times and less dependencies.
    - Vastly lower and more consistent memory usage (RSS and Massif) - before, even with `libmimalloc_sys::mi_collect`, idle usage hovered around ~100 megs and would sometimes spike up to ~500 megs. After, it consistently remains in the area of 15-16 megs.

### September 2, 2023
- Optimized release profile (fat LTO, single codegen unit)
- Added Mac and Windows support (thanks Dan0xE!)
- Add `--wallpaper-command` argument to specify custom action on new wallpaper instead of the default automatic handling (thanks kidanger!)