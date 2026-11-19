#!/usr/bin/env python3
"""Retake every App Store and website screenshot from the current code.

    scripts/screenshots.py                  # all locales, all shots
    scripts/screenshots.py --locale en-US   # one locale
    scripts/screenshots.py --only 03 06     # some shots (by number or name)
    scripts/screenshots.py --no-build       # reuse the last screenshot build

Builds Flowie with the `screenshots` feature (target/screenshots), starts it on a
throwaway database with a 1440×900 window, fills it with demo pages through the
agent socket, puts the window into each state with the `shot.*` methods
(crates/flowie-app/src/shots.rs) and captures it with `screencapture -l`.

Writes
    appstore/fastlane/screenshots/<locale>/NN-name.png   2880×1800 (App Store)
    website/images/NN-name-<locale>.jpg                  1600×1000 (website)

Needs a Retina display (2× scale) and Screen Recording permission for the app
running this script (System Settings → Privacy & Security → Screen Recording).
Your own outline is never touched. Keep the mouse off the Flowie window while
it runs: hover effects would show up in the shots.
"""

import argparse
import datetime as dt
import json
import os
import re
import shutil
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TARGET = ROOT / "target" / "screenshots"
BINARY = TARGET / "release" / "flowie"
APPSTORE = ROOT / "appstore" / "fastlane" / "screenshots"
WEBSITE = ROOT / "website" / "images"
WINDOW = (1440, 900)  # points with the title bar; 2880×1800 pixels on a Retina display
TITLEBAR = 32  # points; FLOWIE_WINDOW_SIZE is the content size
SETTLE = 1.6  # seconds for animations (zoom, flashes, popups, overlay scrollbars) to finish

# --------------------------------------------------------------------- content

TODAY = dt.date.today()


def day(offset):
    return (TODAY + dt.timedelta(days=offset)).isoformat()


def journal_days():
    """Earlier days of this month (so the calendar shows them), newest first."""
    days = [-1, -3, -6, -8, -12]
    return [d for d in days if (TODAY + dt.timedelta(days=d)).month == TODAY.month] or [-1]


EN = {
    "language": "en",
    "journal": "Journal",
    "research": f"""
- Research notes: learning & memory
  - # Learning & memory
  - Spaced repetition beats cramming — **every study we read agrees** #idea
    - Forgetting curve: retention $R = e^{{-t/S}}$, where $S$ is memory stability
    - Review just before you forget; intervals grow about *2.5×* each time
    - Effect is strongest for {{green}}facts and vocabulary{{/}}, weaker for skills
  - ## Key formulas
    - Bayes' rule — how much should new evidence change a belief? $$P(H \\mid E) = \\frac{{P(E \\mid H)\\,P(H)}}{{P(E)}}$$
    - Sanity check from physics class: $E = mc^2$
  - ## This week
    - [x] Read Ebbinghaus (1885), chapter 3
    - [x] Summarize the Cepeda et al. meta-analysis
    - [ ] Draft section 2 and send it to @Maya by {day(1)}
    - [ ] Re-run the retention model on the new data #experiment
  - ## Open questions
    - > The best time to review something is right before you forget it.
    - Does ==interleaving== help with math, or only with vocabulary? #idea
    - {{red}}Sample size is too small{{/}} in study B — ask @Leo about the raw data
    - Compare against the `SM-2` scheduling algorithm #experiment
""",
    "launch": f"""
- Product launch plan
  - # Before launch
    - [x] Freeze features and write the release notes
    - [x] Record the demo video
    - [ ] Final pass on the App Store screenshots — due {day(2)}
    - [ ] Press kit: icon, screenshots, one-paragraph pitch
  - # Launch day
    1. Publish the website and the blog post
    2. Post to the mailing list at 9:00
    3. Answer every comment for the first two hours
  - # Ideas for v1.1
    - Shared pages for small teams #idea
    - Offline sync between Mac and iPad #idea
    - A weekly review that pulls in every open to-do @Maya
""",
    "reading": """
- Reading list
  - [x] *Thinking, Fast and Slow* — Daniel Kahneman
  - [x] *Make It Stick* — Brown, Roediger & McDaniel
  - [ ] *The Art of Doing Science and Engineering* — Richard Hamming
  - [ ] *How to Take Smart Notes* — Sönke Ahrens #idea
  - [ ] *Gödel, Escher, Bach* — Douglas Hofstadter
""",
    "ideas": """
- Ideas
  - A weekly review template that pulls in every open to-do #idea
  - Turn reading highlights into flashcards automatically #idea
  - Map of how all my projects connect #idea
  - Cook through one cuisine per month — starting with Sichuan
""",
    "travel": """
- Travel: Kyoto in spring
  - Book a ryokan near Gion — budget **¥40,000/night**
  - [ ] Check the cherry-blossom forecast in early March
  - Day trip to Nara #idea
""",
    # {research}, {reading}, {travel} are replaced with links to those pages.
    "today": """
- Morning: outlined section 2 of the paper for {research}
- Call with @Maya — she wants the draft by {due}
- [ ] Order more notebooks
- New idea: tag every experiment with its hypothesis #idea
- ## Reading
  - Finished *Make It Stick*, ch. 4 — ==retrieval practice== beats re-reading ({reading})
  - > Learning is deeper and more durable when it's effortful.
- ## Tomorrow
  - [ ] Lab meeting at 10:00 with @Leo
  - [x] Reply to the reviewers
  - [ ] Plan the weekend hike #idea
""",
    "earlier": [
        "Reading group: *Make It Stick*, chapters 1–3",
        "Started the Kyoto itinerary #idea — see {travel}",
        "Ran the pilot study with 12 participants #experiment",
        "Coffee with @Leo about the retention model",
        "Sketched the launch timeline",
    ],
    "slash": "Check whether spacing helps motor skills too /",
    "journal_caret": "New idea",
    "link_text": "Booked the flights for ",
    "link_query": "[[Kyo",
    "palette": "read",
    "search": "#idea",
    "saved_search": "is:todo",
}

ZH = {
    "language": "zh-CN",
    "journal": "日记",
    "research": f"""
- 研究笔记：学习与记忆
  - # 学习与记忆
  - 间隔重复胜过临时抱佛脚 —— **我们读过的研究都这么说** #想法
    - 遗忘曲线：保持率 $R = e^{{-t/S}}$，其中 $S$ 是记忆稳定性
    - 在快要忘记时复习；每次间隔约增长 *2.5 倍*
    - 对{{green}}事实和词汇{{/}}效果最好，对技能较弱
  - ## 关键公式
    - 贝叶斯定理 —— 新证据应当让信念改变多少？ $$P(H \\mid E) = \\frac{{P(E \\mid H)\\,P(H)}}{{P(E)}}$$
    - 物理课上的验算：$E = mc^2$
  - ## 本周
    - [x] 读艾宾浩斯（1885）第三章
    - [x] 总结 Cepeda 等人的元分析
    - [ ] 写完第二节初稿，{day(1)} 前发给 @小林
    - [ ] 用新数据重跑保持率模型 #实验
  - ## 待解决的问题
    - > 复习的最佳时机，是在你快要忘记的时候。
    - ==交错练习==对数学有用，还是只对词汇有用？ #想法
    - B 研究的{{red}}样本量太小{{/}} —— 找 @老王 要原始数据
    - 和 `SM-2` 排程算法比较一下 #实验
""",
    "launch": f"""
- 产品发布计划
  - # 发布前
    - [x] 冻结功能，写好发布说明
    - [x] 录制演示视频
    - [ ] 最后检查一遍 App Store 截图 —— {day(2)} 截止
    - [ ] 媒体资料包：图标、截图、一段话介绍
  - # 发布当天
    1. 上线网站和博客文章
    2. 9:00 发邮件通讯
    3. 前两小时回复每一条评论
  - # v1.1 的想法
    - 小团队共享页面 #想法
    - Mac 与 iPad 离线同步 #想法
    - 每周回顾，自动汇总所有未完成的待办 @小林
""",
    "reading": """
- 阅读清单
  - [x] 《思考，快与慢》—— 丹尼尔·卡尼曼
  - [x] 《认知天性》—— 布朗、罗迪格、麦克丹尼尔
  - [ ] 《学会学习》—— 理查德·汉明
  - [ ] 《卡片笔记写作法》—— 申克·阿伦斯 #想法
  - [ ] 《哥德尔、艾舍尔、巴赫》—— 侯世达
""",
    "ideas": """
- 想法
  - 每周回顾模板，自动汇总所有未完成的待办 #想法
  - 把读书划线自动变成记忆卡片 #想法
  - 画一张所有项目之间关系的图 #想法
  - 每个月学做一种菜系 —— 从川菜开始
""",
    "travel": """
- 旅行：春天去京都
  - 订祇园附近的旅馆 —— 预算 **每晚 4 万日元**
  - [ ] 三月初查一下樱花预报
  - 去奈良一日游 #想法
""",
    "today": """
- 上午：为 {research} 列好第二节的提纲
- 和 @小林 通话 —— 她希望 {due} 前拿到初稿
- [ ] 再买几本笔记本
- 新想法：给每个实验标上它的假设 #想法
- ## 阅读
  - 读完《认知天性》第四章 —— ==提取练习==胜过重读（{reading}）
  - > 费力的学习，才更深入、更持久。
- ## 明天
  - [ ] 10:00 和 @老王 开组会
  - [x] 回复审稿人
  - [ ] 计划周末徒步 #想法
""",
    "earlier": [
        "读书会：《认知天性》第 1–3 章",
        "开始排京都行程 #想法 —— 见 {travel}",
        "完成 12 人的预实验 #实验",
        "和 @老王 喝咖啡聊保持率模型",
        "画了发布时间线",
    ],
    "slash": "看看间隔练习对运动技能是否也有效 /",
    "journal_caret": "新想法",
    "link_text": "订好了机票：",
    "link_query": "[[京都",
    "palette": "阅读",
    "search": "#想法",
    "saved_search": "is:todo",
}

LOCALES = {"en-US": EN, "zh-Hans": ZH}

# ------------------------------------------------------------------ the app


class Flowie:
    def __init__(self, workdir, language, dark=False):
        self.dir = workdir
        self.sock_path = str(workdir / "agent.sock")
        settings = {"agent": "read-write", "language": language, "theme": "system", "accent": "Purple" if dark else ""}
        (workdir / "settings.json").write_text(json.dumps(settings))
        # FLOWIE_SHOT_APPEARANCE sets the app's appearance (title bar included) whatever the
        # Mac is set to; the theme follows it.
        env = dict(os.environ, FLOWIE_DB=str(workdir / "flowie.sqlite"), FLOWIE_SHOT_SOCKET=self.sock_path,
                   FLOWIE_WINDOW_SIZE=f"{WINDOW[0]}x{WINDOW[1] - TITLEBAR}",
                   FLOWIE_SHOT_APPEARANCE="dark" if dark else "light")
        self.log = open(workdir / "flowie.log", "w")
        self.proc = subprocess.Popen([str(BINARY)], env=env, stdout=self.log, stderr=self.log)
        self.sock = None
        for _ in range(100):
            try:
                self.sock = socket.socket(socket.AF_UNIX)
                self.sock.connect(self.sock_path)
                break
            except OSError:
                self.sock.close()
                self.sock = None
                if self.proc.poll() is not None:
                    sys.exit(f"Flowie exited at startup; see {workdir / 'flowie.log'}")
                time.sleep(0.1)
        if not self.sock:
            sys.exit("Flowie's agent socket never came up")
        self.reader = self.sock.makefile("r")
        self.seq = 0
        self.window_id = self.find_window()

    def call(self, method, **params):
        self.seq += 1
        self.sock.sendall((json.dumps({"id": self.seq, "method": method, "params": params}) + "\n").encode())
        resp = json.loads(self.reader.readline())
        if "error" in resp:
            raise RuntimeError(f"{method} {params}: {resp['error']}")
        return resp["result"]

    def create(self, markdown, **where):
        """Create bullets; returns {plain text: id} for every bullet created."""
        out = self.call("create", markdown=markdown.strip("\n"), **where)
        ids = {}
        for line in out.splitlines():
            m = re.match(r"\s*- (.*)  \[id:(\d+)", line)
            if m:
                ids[m.group(1)] = int(m.group(2))
        return ids

    def find_window(self):
        """The CGWindowID of the app's window, via JXA."""
        script = f"""
ObjC.import('CoreGraphics');
const list = ObjC.deepUnwrap(ObjC.castRefToObject($.CGWindowListCopyWindowInfo($.kCGWindowListOptionOnScreenOnly, 0)));
const w = list.find(w => w.kCGWindowOwnerPID === {self.proc.pid} && w.kCGWindowLayer === 0);
w ? String(w.kCGWindowNumber) : '';
"""
        for _ in range(50):
            out = subprocess.run(["osascript", "-l", "JavaScript", "-e", script], capture_output=True, text=True).stdout.strip()
            if out:
                return out
            time.sleep(0.1)
        sys.exit("Couldn't find Flowie's window")

    def capture(self, path):
        time.sleep(SETTLE)
        subprocess.run(["screencapture", "-x", "-o", "-l", self.window_id, str(path)], check=True)

    def quit(self):
        self.proc.terminate()
        try:
            self.proc.wait(5)
        except subprocess.TimeoutExpired:
            self.proc.kill()
        self.log.close()


def link(label, node_id):
    return f"[{label}](flowie:{node_id})"


def first_id(ids, prefix):
    for text, i in ids.items():
        if text.lstrip("#>[x] ").startswith(prefix) or prefix in text:
            return i
    raise KeyError(prefix)


def seed(app, c):
    """Fill the outline with the demo pages; returns ids the shots need."""
    app.call("shot.wipe")
    ids = {}
    for key in ("research", "launch", "reading", "ideas", "travel"):
        made = app.create(c[key])
        ids[key] = next(iter(made.values()))
        ids[key + "/all"] = made
    # Journal: earlier days of the month, then today.
    for d, text in zip(journal_days(), c["earlier"]):
        app.call("journal", date=day(d), markdown="- " + text.format(travel=link(label_of(c["travel"]), ids["travel"])))
    today = c["today"].format(research=link(label_of(c["research"]), ids["research"]),
                              reading=link(label_of(c["reading"]), ids["reading"]), due=day(1))
    ids["today/all"] = app.call("journal", date=day(0), markdown=today.strip("\n"))
    journal = int(re.search(r"under id:(\d+)", ids["today/all"]).group(1))
    ids["today"] = journal
    home = app.call("read", depth=1)
    journal_root = int(re.search(r"- Journal  \[id:(\d+)", home).group(1))
    app.call("update", id=journal_root, text=c["journal"])
    # Collapse the journal days and big pages on Home.
    for d in journal_days():
        m = re.search(rf"- {day(d)}  \[id:(\d+)", app.call("read", id=journal_root, depth=1))
        if m:
            app.call("update", id=int(m.group(1)), collapsed=True)
    for key in ("research", "launch"):
        app.call("update", id=ids[key], collapsed=True)
    for key in ("research", "reading", "launch"):
        app.call("shot.star", id=ids[key])
    app.call("shot.save_search", query=c["saved_search"])
    # Mirror a v1.1 idea into Ideas, so mirrors show up there.
    app.call("shot.mirror", source=first_id(ids["launch/all"], "A weekly review" if c is EN else "每周回顾"),
             parent=ids["ideas"])
    app.call("shot.reset")
    time.sleep(1.5)  # let the creation flashes fade
    return ids


def label_of(markdown):
    return markdown.strip().splitlines()[0].lstrip("- ").strip()


# ------------------------------------------------------------------- shots
# Each shot: (number-name, function(app, content, ids)). The first five keep the
# names the README, the website and the App Store listing already use.


def shot_outline(app, c, ids):
    app.call("shot.zoom", id=ids["research"])
    app.call("shot.caret", id=first_id(ids["research/all"], "# "))


def shot_quick_open(app, c, ids):
    app.call("shot.zoom")
    app.call("shot.palette", query=c["palette"])


def shot_commands(app, c, ids):
    app.call("shot.zoom", id=ids["research"])
    after = first_id(ids["research/all"], "Effect is strongest" if c is EN else "对{green}")
    made = app.create("- x", after=after)
    node = next(iter(made.values()))
    app.call("update", id=node, text="")
    time.sleep(1.2)
    app.call("shot.caret", id=node)
    app.call("shot.type", text=c["slash"])
    return lambda: app.call("delete", ids=[node])


def today_id(ids, prefix):
    """A bullet on today's journal page, by the start of its text."""
    return int(re.search(rf"- (?:\[.\] )?{re.escape(prefix)}.*\[id:(\d+)", ids["today/all"]).group(1))


def shot_journal(app, c, ids):
    app.call("shot.zoom", id=ids["today"])
    app.call("shot.caret", id=today_id(ids, c["journal_caret"]))
    app.call("shot.calendar")


def shot_search(app, c, ids):
    app.call("shot.zoom")
    app.call("shot.search", query=c["search"])


def shot_bullet_menu(app, c, ids):
    app.call("shot.zoom", id=ids["launch"])
    target = first_id(ids["launch/all"], "Final pass" if c is EN else "最后检查")
    # Fourth row of the page (two done to-dos are above it); the menu opens at the
    # pointer, just right of the to-do's checkbox.
    app.call("shot.bullet_menu", id=target, x=80, y=232)


def shot_links(app, c, ids):
    app.call("shot.zoom", id=ids["today"])
    made = app.create("- x", after=today_id(ids, c["journal_caret"]))
    node = next(iter(made.values()))
    app.call("update", id=node, text=c["link_text"])
    time.sleep(1.2)
    app.call("shot.caret", id=node)
    app.call("shot.type", text=c["link_query"])
    return lambda: app.call("delete", ids=[node])


def shot_dark(app, c, ids):
    app.call("shot.zoom", id=ids["launch"])
    app.call("shot.caret", id=first_id(ids["launch/all"], "Shared pages" if c is EN else "小团队"))


def shot_agent(app, c, ids):
    app.call("shot.zoom", id=ids["research"])
    app.call("shot.settings", tab="agent")


# (file name, function, dark appearance)
SHOTS = [
    ("01-outline", shot_outline, False),
    ("02-quick-open", shot_quick_open, False),
    ("03-commands", shot_commands, False),
    ("04-journal-calendar", shot_journal, False),
    ("05-search", shot_search, False),
    ("06-bullet-menu", shot_bullet_menu, False),
    ("07-links", shot_links, False),
    ("08-dark", shot_dark, True),
    ("09-agent", shot_agent, False),
]

# --------------------------------------------------------------------- main


def check_display():
    out = subprocess.run(["system_profiler", "SPDisplaysDataType"], capture_output=True, text=True).stdout
    if "Retina" not in out and not re.search(r"UI Looks like: (\d+) x \d+", out):
        print("warning: couldn't check the display; shots need a 2× (Retina) display", file=sys.stderr)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--locale", choices=sorted(LOCALES), action="append", help="only this locale (repeatable)")
    ap.add_argument("--only", nargs="+", metavar="SHOT", help="only these shots, by number (03) or name (commands)")
    ap.add_argument("--no-build", action="store_true", help="reuse target/screenshots")
    ap.add_argument("--keep", action="store_true", help="keep the temporary database and log")
    args = ap.parse_args()

    shots = [s for s in SHOTS if not args.only or any(s[0].startswith(o) or s[0].endswith(o) for o in args.only)]
    if not shots:
        sys.exit("no shot matches --only")
    if not args.no_build:
        subprocess.run(["cargo", "build", "--release", "-p", "flowie-app", "--features", "screenshots",
                        "--target-dir", str(TARGET)], cwd=ROOT, check=True)
    check_display()

    for locale in args.locale or list(LOCALES):
        (APPSTORE / locale).mkdir(parents=True, exist_ok=True)
        for dark in (False, True):
            group = [s for s in shots if s[2] == dark]
            if group:
                run_session(locale, dark, group, args.keep)


def run_session(locale, dark, shots, keep):
    """One app run: seed the outline, then take `shots` in order."""
    c = LOCALES[locale]
    # Short path: Unix socket paths are limited to ~100 bytes.
    work = Path(tempfile.mkdtemp(prefix="flowie-shots-", dir="/tmp"))
    app = Flowie(work, c["language"], dark)
    try:
        ids = seed(app, c)
        for name, fn, _ in shots:
            app.call("shot.reset")
            undo = fn(app, c, ids)
            png = APPSTORE / locale / f"{name}.png"
            app.capture(png)
            w, h = image_size(png)
            if (w, h) != (WINDOW[0] * 2, WINDOW[1] * 2):
                print(f"warning: {png.name} is {w}×{h}, not {WINDOW[0] * 2}×{WINDOW[1] * 2} (window clipped or not 2×?)",
                      file=sys.stderr)
            jpg = WEBSITE / f"{name}-{locale}.jpg"
            subprocess.run(["sips", "-s", "format", "jpeg", "-s", "formatOptions", "82", "-Z", "1600", str(png),
                            "--out", str(jpg)], check=True, capture_output=True)
            print(f"{locale}  {name}")
            app.call("shot.reset")
            if undo:
                undo()
    finally:
        app.quit()
        if keep:
            print(f"kept {work}")
        else:
            shutil.rmtree(work, ignore_errors=True)


def image_size(path):
    out = subprocess.run(["sips", "-g", "pixelWidth", "-g", "pixelHeight", str(path)], capture_output=True, text=True).stdout
    return tuple(int(x) for x in re.findall(r"pixel(?:Width|Height): (\d+)", out))


if __name__ == "__main__":
    main()
