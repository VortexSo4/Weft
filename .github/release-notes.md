**Install:** unpack the archive for your system anywhere and run the game. Needs a GPU with Vulkan.

- **Windows:** run `weft.exe`.
- **Linux (x64):** `tar xf Weft-*-linux-x64.tar.gz`, then `Weft/weft`. Needs your GPU's Vulkan driver.
- **macOS (Apple Silicon, macOS 12 or newer):** `tar xf Weft-*-macos-arm64.tar.gz`, then `xattr -dr com.apple.quarantine Weft`, as the game is not signed by Apple, then `Weft/weft`. Vulkan comes with the game (MoltenVK).

**Textures:** the game comes with its own. If Minecraft is installed through the official launcher or Prism, Weft builds a resource pack from your copy on the first start (Settings > Resource Packs to turn it off). Nothing from Mojang is included or downloaded.

**Multiplayer:** pause menu > Multiplayer > Open to Friends gives a ticket; your friend pastes it into Multiplayer > Join. It also works by address, with the port forwarded, and `weft --server` runs a world with no window (see the README).

**Mods:** put a mod's `.zip` into `run/mods/`. Mods are native libraries and have to be built for your system and the same engine version, see the [modding cheat sheet](https://github.com/Weft-Rust/Weft/blob/master/docs/modding.md).

The mod interface and the save format can still change between versions.

