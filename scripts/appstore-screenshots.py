#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.10"
# dependencies = ["Pillow>=11.0.0"]
# ///
"""Frame the app captures (screenshots-source/, from scripts/screenshots.py) as bilingual
App Store screenshots (screenshots/, 2880×1800): a centered headline over the window."""

from __future__ import annotations

from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter, ImageFont

ROOT = Path(__file__).resolve().parent.parent
SCREENSHOTS = ROOT / "appstore/fastlane/screenshots"
SOURCES = ROOT / "appstore/fastlane/screenshots-source"
SIZE = (2880, 1800)
EN_FONT = "/System/Library/Fonts/Supplemental/Avenir Next.ttc"
ZH_FONT = "/System/Library/Fonts/Hiragino Sans GB.ttc"

COPY = {
    "en-US": {
        "01-outline": ("Make space for", "the big picture.", "A clear, flexible home for everything on your mind."),
        "02-quick-open": ("Your next idea,", "one shortcut away.", "Jump to any page or start a new one with ⌘P."),
        "03-commands": ("Stay in flow.", "Leave the menus behind.", "Create, format and organize without reaching for the mouse."),
        "04-journal-calendar": ("A little more", "intentional, daily.", "Your journal and calendar, connected to everything else."),
        "05-search": ("Every open to-do,", "from every page.", "Search by tags, tasks, dates and the details you remember."),
        "06-focus": ("Go deep,", "never lose your place.", "The title tucks away as you scroll, and the path shows where you are."),
        "07-links": ("One thought", "leads to another.", "Connect pages and follow the links between your ideas."),
        "08-dark": ("A workspace", "that feels like yours.", "Set the mood with dark mode and a color you love."),
        "09-agent": ("Bring AI into", "your train of thought.", "Connect an assistant. You decide what it can access."),
        "10-images": ("Formulas and pictures,", "right in your notes.", "LaTeX math and captioned images, next to the idea they explain."),
    },
    "zh-Hans": {
        "01-outline": ("把思路展开，", "看见全貌。", "清晰灵活的大纲，让想法各有位置。"),
        "02-quick-open": ("下一条想法，", "一键就出现。", "⌘P 直达任意页面，也可以马上开始新想法。"),
        "03-commands": ("思路不停，", "双手不离键盘。", "创建、排版、整理，不必切换到鼠标。"),
        "04-journal-calendar": ("每天留一点，", "给自己的时间。", "日记与日历相连，也与所有想法相连。"),
        "05-search": ("所有未完成的待办，", "一次全部找到。", "按标签、待办、日期和记得的细节筛选。"),
        "06-focus": ("越钻越深，", "也不会迷路。", "滚动时标题收进顶栏，路径告诉你身在何处。"),
        "07-links": ("一个想法，", "自然连向另一个。", "连接页面，沿着思路之间的线索继续探索。"),
        "08-dark": ("工作空间，", "就该合你心意。", "深色模式与喜欢的强调色，营造专属氛围。"),
        "09-agent": ("让 AI 加入，", "由你掌握边界。", "连接智能助手，并决定它可以访问什么。"),
        "10-images": ("公式和图片，", "就在笔记里。", "LaTeX 公式与带说明的图片，紧挨着它们解释的想法。"),
    },
}

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


def font(path: str, size: int, index: int = 0) -> ImageFont.FreeTypeFont:
    return ImageFont.truetype(path, size, index=index)


def wrap_text(text: str, draw: ImageDraw.ImageDraw, selected_font: ImageFont.FreeTypeFont,
              max_width: int, chinese: bool) -> list[str]:
    units = list(text) if chinese else text.split()
    lines: list[str] = []
    current = ""
    for unit in units:
        candidate = current + unit if chinese else (current + " " + unit).strip()
        if current and draw.textlength(candidate, font=selected_font) > max_width:
            lines.append(current)
            current = unit
        else:
            current = candidate
    if current:
        lines.append(current)
    return lines


def rounded_window(image: Image.Image, dark: bool, width: int) -> Image.Image:
    """The capture scaled to `width`, with rounded corners and a hairline edge."""
    height = round(image.height * width / image.width)
    image = image.resize((width, height), Image.Resampling.LANCZOS)
    radius = 30
    mask = Image.new("L", image.size, 0)
    ImageDraw.Draw(mask).rounded_rectangle((0, 0, *image.size), radius=radius, fill=255)
    framed = Image.new("RGBA", image.size, (255, 255, 255, 0))
    framed.paste(image.convert("RGBA"), (0, 0), mask)
    ImageDraw.Draw(framed).rounded_rectangle((1, 1, width - 2, height - 2), radius=radius,
                                             outline=(120, 132, 150, 150) if dark else (20, 30, 40, 40), width=2)
    return framed


def compose(source: Path, destination: Path, locale: str, key: str, index: int) -> None:
    title_a, title_b, subtitle = COPY[locale][key]
    chinese = locale == "zh-Hans"
    dark = key == "08-dark"
    background, accent = PALETTES[index % len(PALETTES)]
    if dark:
        background, accent = (24, 27, 36), (167, 139, 250)
    ink = (246, 247, 250) if dark else (23, 35, 39)
    secondary = (190, 198, 210) if dark else (72, 86, 90)
    # Demi Bold / W6 headline, Regular / W3 subtitle.
    display = font(ZH_FONT if chinese else EN_FONT, 100 if chinese else 96, 2)
    detail = font(ZH_FONT if chinese else EN_FONT, 42 if chinese else 40, 0 if chinese else 7)

    canvas = Image.new("RGBA", SIZE, background + (255,))
    draw = ImageDraw.Draw(canvas)
    # Headline: two centered lines, the second in the accent color, then the subtitle.
    center = SIZE[0] // 2
    draw.text((center, 120), title_a, font=display, fill=ink, anchor="ma")
    draw.text((center, 240), title_b, font=display, fill=accent, anchor="ma")
    draw.text((center, 384), subtitle, font=detail, fill=secondary, anchor="ma")

    # The window, large and centered, running off the bottom edge.
    window = rounded_window(Image.open(source), dark, 2240)
    x, y = (SIZE[0] - window.width) // 2, 500
    shadow = Image.new("RGBA", window.size, (10, 20, 30, 0))
    shadow.putalpha(window.getchannel("A").filter(ImageFilter.GaussianBlur(36)).point(lambda p: int(p * (0.45 if dark else 0.22))))
    canvas.alpha_composite(shadow, (x, y + 16))
    canvas.alpha_composite(window, (x, y))
    canvas.convert("RGB").save(destination, format="PNG", optimize=True)


def main() -> None:
    for locale, copy in COPY.items():
        output_dir = SCREENSHOTS / locale
        output_dir.mkdir(parents=True, exist_ok=True)
        for index, key in enumerate(copy):
            compose(SOURCES / locale / f"{key}.png", output_dir / f"{key}.png", locale, key, index)
            print(f"{locale}/{key}.png")


if __name__ == "__main__":
    main()
