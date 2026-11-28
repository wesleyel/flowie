"""The palette and headline typography of Flowie's App Store screenshots."""

from pathlib import Path

from PIL import ImageFont

INK = (23, 35, 39)
SECONDARY = (72, 86, 90)

# Background and headline accent, in Mac screenshot order.
PALETTES = [
    ((231, 244, 236), (31, 104, 78)),
    ((239, 237, 250), (86, 72, 151)),
    ((250, 239, 222), (159, 94, 33)),
    ((224, 241, 243), (31, 111, 127)),
    ((247, 231, 232), (157, 63, 75)),
    ((233, 239, 249), (57, 93, 153)),
    ((241, 237, 225), (131, 99, 43)),
    ((231, 238, 228), (73, 111, 64)),
    ((239, 231, 245), (112, 69, 135)),
    ((247, 237, 222), (155, 91, 34)),
]


def typefaces(locale: str, title_size: int, detail_size: int):
    """Demi Bold / W6 headline and Regular / W3 subtitle, from macOS's font collections."""
    chinese = locale == "zh-Hans"
    name = "Hiragino Sans GB.ttc" if chinese else "Avenir Next.ttc"
    fonts = Path("/System/Library/Fonts")
    for path in (fonts / name, fonts / "Supplemental" / name):
        if path.is_file():
            return (ImageFont.truetype(str(path), title_size, index=2),
                    ImageFont.truetype(str(path), detail_size, index=0 if chinese else 7))
    raise FileNotFoundError(f"Framing needs the macOS system font {name} in {fonts} or Supplemental")
