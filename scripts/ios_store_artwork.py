"""Frame iOS Simulator captures for the store. Run scripts/ios-screenshots.py --frame-only."""

import tempfile
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter

from appstore_style import INK, PALETTES, SECONDARY, typefaces

COPY = {
    "en-US": {
        "home": ("Make space for", "the big picture.", "Keep plans, notes and to-dos in one outline."),
        "today": ("Give each day", "a page of its own.", "Keep a daily log and link back to your projects."),
        "page": ("Tap a bullet.", "Give it a page.", "Focus on one branch of your outline."),
        "panes": ("Two pages,", "side by side.", "Keep the project in view as you open a detail."),
        "search": ("One tag.", "Every matching note.", "Find ideas across projects, lists and journal entries."),
        "capture": ("A thought to keep?", "Jot it down.", "Open today’s journal straight from Control Center."),
    },
    "zh-Hans": {
        "home": ("把思路展开，", "看见全貌。", "计划、笔记和待办，都在同一份大纲里。"),
        "today": ("一天一页，", "写下今天的事。", "在日记里记下进展，链接到相关项目。"),
        "page": ("轻点圆点，", "专注这一页。", "单独打开一个分支，标题下的路径标出它的位置。"),
        "panes": ("项目和细节，", "并排看。", "横屏点开条目，上一页就留在旁边。"),
        "search": ("一个标签，", "找齐相关笔记。", "搜遍项目、清单和日记里的条目。"),
        "capture": ("想到了，", "就记下来。", "从控制中心打开今天的日记，直接输入。"),
    },
}

# Match the Mac's outline, journal, focus and search colors, regardless of shot order.
COLORS = {"home": PALETTES[0], "today": PALETTES[3], "page": PALETTES[5],
          "search": PALETTES[4], "capture": PALETTES[2]}


def place_device(canvas: Image.Image, capture: Image.Image, phone: bool) -> None:
    """Fit the whole display, including the tab bar or keyboard, in a concentric bezel."""
    width = 1112 if phone else 2144
    bezel = round(width * (0.042 if phone else 0.026))
    radius = round(width * (0.156 if phone else 0.038))
    screen_w = width - 2 * bezel
    screen_h = round(screen_w * capture.height / capture.width)
    height = screen_h + 2 * bezel
    x = (canvas.width - width) // 2
    y = canvas.height - (96 if phone else 56) - height

    shadow = Image.new("L", canvas.size)
    ImageDraw.Draw(shadow).rounded_rectangle((x, y + 16, x + width - 1, y + height + 15), radius, fill=58)
    canvas.paste((27, 37, 42), (0, 0), shadow.filter(ImageFilter.GaussianBlur(25)))
    draw = ImageDraw.Draw(canvas)
    for inset, color in ((0, "#565b60"), (2, "#b7babd"), (5, "#121416")):
        draw.rounded_rectangle((x + inset, y + inset, x + width - 1 - inset, y + height - 1 - inset),
                               radius - inset, fill=color)

    screen = capture.convert("RGB").resize((screen_w, screen_h), Image.Resampling.LANCZOS)
    mask = Image.new("L", screen.size)
    ImageDraw.Draw(mask).rounded_rectangle((0, 0, screen_w - 1, screen_h - 1), radius - bezel, fill=255)
    canvas.paste(screen, (x + bezel, y + bezel), mask)
    if phone:
        # simctl omits the physical Dynamic Island. Same safe-area proportions as website/style.css.
        iw, ih = round(screen_w * 0.286), round(screen_w * 0.084)
        left, top = (canvas.width - iw) // 2, y + bezel + round(screen_w * 0.025)
        draw.rounded_rectangle((left, top, left + iw - 1, top + ih - 1), ih // 2, fill="#050506")


def compose(source: Path, destination: Path, locale: str, name: str, kind: str, expected: tuple[int, int]) -> None:
    phone = kind == "iphone"
    title, accent_title, subtitle = COPY[locale]["panes" if not phone and name == "page" else name]
    bg, accent = COLORS[name]
    with Image.open(source) as capture:
        if capture.format != "PNG" or capture.size != expected:
            raise ValueError(f"{source}: expected a {expected[0]}×{expected[1]} PNG Simulator capture")
        canvas = Image.new("RGB", expected, bg)
        place_device(canvas, capture, phone)

    display, detail = typefaces(locale, 106 if phone else 98, 46 if phone else 44)
    baselines = (178, 296, 394) if phone else (123, 229, 313)
    draw = ImageDraw.Draw(canvas)
    for text, face, color, y in zip((title, accent_title, subtitle), (display, display, detail),
                                    (INK, accent, SECONDARY), baselines):
        center = (canvas.width // 2, y)
        left, _, right, _ = draw.textbbox(center, text, font=face, anchor="ms")
        if left < 60 or right > canvas.width - 60:
            raise ValueError(f"{locale}/{kind}/{name}: line too wide: {text}")
        draw.text(center, text, font=face, fill=color, anchor="ms")

    destination.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=destination.parent) as tmp:
        png = Path(tmp) / destination.name
        canvas.save(png, format="PNG", optimize=True)
        png.replace(destination)
