#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.10"
# dependencies = ["Pillow>=11.0.0"]
# ///
"""Compose bilingual App Store screenshots from the existing app captures."""

from __future__ import annotations

import shutil
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter, ImageFont

ROOT = Path(__file__).resolve().parent.parent
SCREENSHOTS = ROOT / "appstore/fastlane/screenshots"
SOURCES = ROOT / "appstore/fastlane/screenshots-source"
SIZE = (2880, 1800)
EN_FONT = "/System/Library/Fonts/Supplemental/Avenir Next.ttc"
ZH_FONT = "/System/Library/Fonts/Hiragino Sans GB.ttc"
ICON = ROOT / "website/images/icon.png"

COPY = {
    "en-US": {
        "01-outline": ("Make space for", "the big picture.", "A clear, flexible home for everything on your mind."),
        "02-quick-open": ("Your next idea,", "one shortcut away.", "Jump to any page or start a new one with ⌘P."),
        "03-commands": ("Stay in flow.", "Leave the menus behind.", "Create, format and organize without reaching for the mouse."),
        "04-journal-calendar": ("A little more", "intentional, daily.", "Your journal and calendar, connected to everything else."),
        "05-search": ("Find the thought", "behind the words.", "Search by tags, tasks, dates and the details you remember."),
        "06-bullet-menu": ("Ideas change.", "Your outline can too.", "Move, shape and reorganize thoughts as they take form."),
        "07-links": ("One thought", "leads to another.", "Connect pages and follow the links between your ideas."),
        "08-dark": ("A workspace", "that feels like yours.", "Set the mood with dark mode and a color you love."),
        "09-agent": ("Bring AI into", "your train of thought.", "Connect an assistant. You decide what it can access."),
        "10-images": ("Keep the whole", "story together.", "Add images and captions right where the idea lives."),
    },
    "zh-Hans": {
        "01-outline": ("把思路展开，", "看见全貌。", "清晰灵活的大纲，让想法各有位置。"),
        "02-quick-open": ("下一条想法，", "一键就出现。", "⌘P 直达任意页面，也可以马上开始新想法。"),
        "03-commands": ("思路不停，", "双手不离键盘。", "创建、排版、整理，不必切换到鼠标。"),
        "04-journal-calendar": ("每天留一点，", "给自己的时间。", "日记与日历相连，也与所有想法相连。"),
        "05-search": ("搜到关键词，", "也搜到那件事。", "按标签、待办、日期和记得的细节筛选。"),
        "06-bullet-menu": ("想法在变化，", "大纲也能跟上。", "随时移动、调整、重组正在成形的思路。"),
        "07-links": ("一个想法，", "自然连向另一个。", "连接页面，沿着思路之间的线索继续探索。"),
        "08-dark": ("工作空间，", "就该合你心意。", "深色模式与喜欢的强调色，营造专属氛围。"),
        "09-agent": ("让 AI 加入，", "由你掌握边界。", "连接智能助手，并决定它可以访问什么。"),
        "10-images": ("让想法的故事，", "完整留在一起。", "在内容旁添加图片与说明，让细节不丢失。"),
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


def rounded_window(image: Image.Image, tilt: float, dark: bool, size: tuple[int, int]) -> Image.Image:
    image = image.resize(size, Image.Resampling.LANCZOS)
    mask = Image.new("L", image.size, 0)
    ImageDraw.Draw(mask).rounded_rectangle((0, 0, *image.size), radius=26, fill=255)
    framed = Image.new("RGBA", image.size, (255, 255, 255, 0))
    framed.paste(image.convert("RGBA"), (0, 0), mask)
    edge = ImageDraw.Draw(framed)
    edge.rounded_rectangle((1, 1, image.width - 2, image.height - 2), radius=26,
                           outline=(166, 184, 200, 170) if dark else (255, 255, 255, 220), width=3)
    return framed.rotate(tilt, resample=Image.Resampling.BICUBIC, expand=True)


def compose(source: Path, destination: Path, locale: str, key: str, index: int) -> None:
    title_a, title_b, subtitle = COPY[locale][key]
    chinese = locale == "zh-Hans"
    dark = key == "08-dark"
    background, accent = PALETTES[index % len(PALETTES)]
    if dark:
        background, accent = (25, 29, 38), (49, 77, 91)
    canvas = Image.new("RGB", SIZE, background)
    draw = ImageDraw.Draw(canvas)
    display = font(ZH_FONT if chinese else EN_FONT, 104 if chinese else 100)
    detail = font(ZH_FONT if chinese else EN_FONT, 39 if chinese else 37)
    wordmark = font(EN_FONT, 58)
    ink = (248, 248, 250) if dark else (23, 35, 39)
    secondary = (198, 207, 218) if dark else (65, 82, 84)

    # A shallow angled masthead gives the composition motion without touching the headline.
    masthead = (18, 22, 30) if dark else accent
    draw.polygon([(0, 0), (SIZE[0], 0), (SIZE[0], 150), (0, 245)], fill=masthead)
    brand_icon = Image.open(ICON).convert("RGBA").resize((88, 88), Image.Resampling.LANCZOS)
    canvas.paste(brand_icon, (142, 48), brand_icon)
    draw.text((250, 58), "FLOWIE", font=wordmark, fill=(255, 255, 255))

    # Alternate balanced two-column compositions; keep the capture large and readable.
    image_left = index % 2 == 0
    tilt = 0.65 if index % 4 < 2 else -0.65
    image_size = (1740, 1088)
    window = rounded_window(Image.open(source), tilt, dark, image_size)
    x = 85 if image_left else SIZE[0] - 85 - window.width
    y = 510

    text_width, text_height = 900, 760
    text_layer = Image.new("RGBA", (text_width, text_height), (0, 0, 0, 0))
    text_draw = ImageDraw.Draw(text_layer)
    text_anchor = "ra" if not image_left else "la"
    text_edge = text_width - 10 if not image_left else 10
    text_top = 180
    title_lines = wrap_text(title_a, text_draw, display, text_width - 40, chinese)
    title_lines.extend(wrap_text(title_b, text_draw, display, text_width - 40, chinese))
    title_y = text_top
    for line in title_lines:
        text_draw.text((text_edge, title_y), line, font=display, fill=ink, anchor=text_anchor)
        title_y += 128
    subtitle_y = title_y + 34
    subtitle_lines = wrap_text(subtitle, text_draw, detail, text_width - 40, chinese)
    text_draw.multiline_text((text_edge, subtitle_y), "\n".join(subtitle_lines), font=detail, fill=secondary,
                             anchor=text_anchor, spacing=13, align="right" if not image_left else "left")
    marker_left = text_edge - 78 if not image_left else text_edge
    text_draw.rounded_rectangle((marker_left, subtitle_y + 125, marker_left + 78, subtitle_y + 135),
                                radius=5, fill=accent)
    text_layer = text_layer.rotate(tilt, resample=Image.Resampling.BICUBIC, expand=True)
    text_x = SIZE[0] - 100 - text_layer.width if image_left else 100
    text_y = 625

    shadow = Image.new("RGBA", window.size, (16, 28, 36, 0))
    shadow.putalpha(window.getchannel("A").filter(ImageFilter.GaussianBlur(42)).point(lambda p: int(p * 0.28)))
    canvas_rgba = canvas.convert("RGBA")
    canvas_rgba.alpha_composite(shadow, (x + 5, y + 20))
    canvas_rgba.alpha_composite(window, (x, y))
    canvas_rgba.alpha_composite(text_layer, (text_x, text_y))
    canvas = canvas_rgba.convert("RGB")
    canvas.save(destination, format="PNG", optimize=True)


def main() -> None:
    for locale, copy in COPY.items():
        source_dir = SOURCES / locale
        source_dir.mkdir(parents=True, exist_ok=True)
        output_dir = SCREENSHOTS / locale
        for index, (key, _) in enumerate(copy.items()):
            filename = f"{key}.png"
            original = source_dir / filename
            output = output_dir / filename
            if not original.exists():
                shutil.copy2(output, original)
            compose(original, output, locale, key, index)
            print(f"{locale}/{filename}")


if __name__ == "__main__":
    main()
