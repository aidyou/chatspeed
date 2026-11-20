"""Build the native icon font used by the GPUI workflow observer.

The web build consumes the iconfont.cn export directly, but GPUI's cosmic-text
backend drops every face whose charmap has no Latin `m` from the primary-font
candidates (the text editor measures layout with that glyph). A pure
private-use-area icon font would therefore be replaced by a fallback face and
every icon would render as an unrelated glyph.

The native copy therefore adds:

* an empty `m` glyph so the face is accepted as a primary font;
* `double-arrow-up` at U+F101, a vertical mirror of `double-arrow-down`, so
  collapsed sections can show the rotated chevron the Vue list renders with a
  CSS transform.

It also synchronizes the left side bearings with the rebuilt glyph bounds so
GPUI preserves the horizontal placement of the original icon outlines.

Usage: python3 scripts/build_gpui_icon_font.py
"""

from pathlib import Path

from fontTools.misc.transform import Transform
from fontTools.pens.transformPen import TransformPen
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.ttLib import TTFont
from fontTools.ttLib.tables._g_l_y_f import Glyph

ROOT = Path(__file__).resolve().parent.parent
SOURCE = ROOT / "src/components/icon/iconfont.ttf"
TARGET = ROOT / "src-tauri/crates/gpui/assets/chatspeed-icons.ttf"

UP_ARROW_CODEPOINT = 0xF101
UP_ARROW_NAME = "double-arrow-up"
MIRRORED_GLYPH = "double-arrow-down"


def add_latin_m(font):
    if "space.m" in font.getGlyphOrder():
        return
    font["glyf"].glyphs["space.m"] = Glyph()
    font["glyf"].glyphs["space.m"].numberOfContours = 0
    font["hmtx"].metrics["space.m"] = (512, 0)
    font.setGlyphOrder(font.getGlyphOrder() + ["space.m"])
    for table in font["cmap"].tables:
        if table.isUnicode():
            table.cmap[ord("m")] = "space.m"


def preserve_horizontal_origins(font):
    """Preserve the original pen origin when FontTools rebuilds glyf bounds."""
    glyf = font["glyf"]
    metrics = font["hmtx"].metrics
    # Snapshot all bounds before recalculating: a composite can also update
    # its components. Swash shifts the outline by hmtx.lsb - glyf.xMin.
    old_bounds = {
        name: glyph.xMin
        for name in font.getGlyphOrder()
        if (glyph := glyf[name]).numberOfContours
    }
    for name, old_x_min in old_bounds.items():
        glyph = glyf[name]
        glyph.recalcBounds(glyf)
        advance, bearing = metrics[name]
        metrics[name] = (advance, bearing + glyph.xMin - old_x_min)


def add_mirrored_arrow(font):
    if UP_ARROW_NAME in font.getGlyphOrder():
        return
    glyph_set = font.getGlyphSet()
    glyf = font["glyf"]
    down = glyf[MIRRORED_GLYPH]
    transform = Transform(1, 0, 0, -1, 0, down.yMin + down.yMax)
    pen = TTGlyphPen(glyph_set)
    # Mirror raw contours, not the glyph-set drawing after its bearing offset.
    down.draw(TransformPen(pen, transform), glyf)
    glyf.glyphs[UP_ARROW_NAME] = pen.glyph()
    font["hmtx"].metrics[UP_ARROW_NAME] = font["hmtx"].metrics[MIRRORED_GLYPH]
    font.setGlyphOrder(font.getGlyphOrder() + [UP_ARROW_NAME])
    for table in font["cmap"].tables:
        if table.isUnicode():
            table.cmap[UP_ARROW_CODEPOINT] = UP_ARROW_NAME


def check_horizontal_origins(source, native):
    for name in source.getGlyphOrder():
        old = source["glyf"][name]
        if not old.numberOfContours:
            continue
        new = native["glyf"][name]
        advance, bearing = source["hmtx"].metrics[name]
        expected = (advance, bearing + new.xMin - old.xMin)
        assert native["hmtx"].metrics[name] == expected, f"pen origin moved for {name}"


def main():
    font = TTFont(SOURCE, recalcTimestamp=False)
    preserve_horizontal_origins(font)
    add_latin_m(font)
    add_mirrored_arrow(font)
    TARGET.parent.mkdir(parents=True, exist_ok=True)
    font.save(TARGET)

    with TTFont(SOURCE) as source, TTFont(TARGET) as check:
        check_horizontal_origins(source, check)
        cmap = check.getBestCmap()
        assert cmap[ord("m")] == "space.m", "native icon font lost its Latin 'm'"
        assert cmap[UP_ARROW_CODEPOINT] == UP_ARROW_NAME, "native icon font lost double-arrow-up"
        assert check["hmtx"].metrics[UP_ARROW_NAME] == check["hmtx"].metrics[MIRRORED_GLYPH]
        assert check["glyf"][UP_ARROW_NAME].xMin == check["glyf"][MIRRORED_GLYPH].xMin
        icons = sum(1 for code in cmap if 0xE000 <= code <= 0xF8FF)
        print(f"{TARGET.relative_to(ROOT)}: glyphs={check['maxp'].numGlyphs} icons={icons}")


if __name__ == "__main__":
    main()