# TF3 Map Generator - Quick Tutorial

Turn a real place on Earth into a playable Transport Fever 3 map.

## What you need

- `tf3-mapgen-gui.exe` (one file, no installation)
- Transport Fever 3 installed
- Internet connection while generating

## Step 1 - Generate the map files

1. Double-click **`tf3-mapgen-gui.exe`**.
2. Paste a location:
   - a Google Maps link (copy from your browser), or
   - a place name like `Interlaken, Switzerland`, or
   - coordinates like `51.15, 14.99`
3. Check the **Paths** section (already auto-detected for Steam):
   - **Game folder** - your TF3 installation
   - **Userdata folder** - where the map files are written
   - Leave **Output folder** empty (or set it to test somewhere else first)
4. Pick **Size** and **Ratio** (default `Medium 1:1`). Keep the first map small-ish (`Medium` is fine; `Small` is faster).
5. Click **Generate map** and wait (a few seconds). When done you'll see a preview
   and a summary like: `13 towns | 171..409 m | water 182 m`.

Tip: for **bigger towns at the start**, create the map later on with the
**Population** setting at 200% in the Map Editor's New Map screen (see step 2).
Towns also grow over the course of the game.

## Step 2 - Import it into the game

The map can't be loaded directly - TF3 imports it through the Map Editor.
You need to be at the **main menu** (not in a running game).

1. **Map Editor -> New Map** -> climate **Temperate**, size and ratio **exactly as
   shown in the app log** (e.g. `Medium` / `1:1`) -> start the editor.

2. **Heightmap tool** (mountain-with-arrow icon in the toolbar) -> **Import** tab:
   - select **`<name>.png`**
   - enter the exact numbers from the app log / `settings.txt`:
     **Minimum Height**, **Maximum Height**, **Water Level**
   - click **Import** and wait.

3. *(optional, recommended)* Same window -> **Biomes** tab:
   - select `<name>_rivers.png`, `<name>_mountains.png` and `<name>_biomes.png`
   - click **Apply** (these steer river/mountain vegetation).

4. **Towns & Industries** tool -> **Import** tab -> select **`<name>.lua`** ->
   **Import**. The real towns get built with houses.

5. **Street connections** tool -> **Propose Default Connections** -> **Generate**
   (creates the road network between towns).

6. Pause menu -> **Save Map** (give it a name).

## Step 3 - Play it

- Main menu -> **Load Game** -> pick your saved map -> configure the game and start.
- Towns start small and grow as you connect them - that's normal for TF3.

## If something looks wrong

- **Map looks mirrored**: tick **Flip north/south** (and/or east/west) in the
  Advanced section, generate again and re-import. Compare with `preview.png`.
- **Experimental sizes** (Tiny/Huge/...): set `experimentalMapFeatures = true`
  in `settings.lua` in the userdata folder first.
- **Anything fails**: an error dialog appears (and the app has an **Open log
  file** button). The log `tf3-mapgen.log` contains everything needed to debug.

## Handy files the app writes

| File | Where | Purpose |
|---|---|---|
| `<name>.png` | `heightmaps/` | the terrain |
| `<name>_rivers.png` etc. | `biomes/` | vegetation masks |
| `<name>.lua` | `towns_industries/` | town positions |
| `preview.png`, `settings.txt` | `tf3_mapgen/<name>/` | check before importing |
