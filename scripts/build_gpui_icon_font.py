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
TARGET = ROOT / "src-tauri/crates/gpui-observer/assets/chatspeed-icons.ttf"

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


def add_mirrored_arrow(font):
    if UP_ARROW_NAME in font.getGlyphOrder():
        return
    glyph_set = font.getGlyphSet()
    down = font["glyf"][MIRRORED_GLYPH]
    down.recalcBounds(font["glyf"])
    transform = Transform(1, 0, 0, -1, 0, down.yMin + down.yMax)
    pen = TTGlyphPen(glyph_set)
    glyph_set[MIRRORED_GLYPH].draw(TransformPen(pen, transform))
    font["glyf"].glyphs[UP_ARROW_NAME] = pen.glyph()
    font["hmtx"].metrics[UP_ARROW_NAME] = font["hmtx"].metrics[MIRRORED_GLYPH]
    font.setGlyphOrder(font.getGlyphOrder() + [UP_ARROW_NAME])
    for table in font["cmap"].tables:
        if table.isUnicode():
            table.cmap[UP_ARROW_CODEPOINT] = UP_ARROW_NAME


def main():
    font = TTFont(SOURCE)
    add_latin_m(font)
    add_mirrored_arrow(font)
    TARGET.parent.mkdir(parents=True, exist_ok=True)
    font.save(TARGET)

    check = TTFont(TARGET)
    cmap = check.getBestCmap()
    assert cmap[ord("m")] == "space.m", "native icon font lost its Latin 'm'"
    assert cmap[UP_ARROW_CODEPOINT] == UP_ARROW_NAME, "native icon font lost double-arrow-up"
    icons = sum(1 for code in cmap if 0xE000 <= code <= 0xF8FF)
    print(f"{TARGET.relative_to(ROOT)}: glyphs={check['maxp'].numGlyphs} icons={icons}")


if __name__ == "__main__":
    main()