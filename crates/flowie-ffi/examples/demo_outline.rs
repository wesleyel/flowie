//! The demo outline of the iOS App Store screenshots (scripts/ios-screenshots.py), written
//! through the same calls the app makes. Never the user's outline: it writes a new database.
//!
//!     cargo run -p flowie-ffi --example demo_outline -- <db path> <en-US|zh-Hans> <today YYYY-MM-DD>
//!
//! Prints the ids the screenshots open, as JSON: `{"web": "…", "milestones": "…", …}`.

use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::{Days, NaiveDate};
use flowie_ffi::{FlowieError, Outline};

/// The outline as text: two spaces a level, `- ` before each bullet. A line can start with
/// `[x] ` (completed) and end with flags: ` ^star`, ` ^fold` (collapsed), ` ^key=name` (print
/// its id as `name`), and last ` ^note=…`. `{name}` in a line is a link to the bullet named so.
/// `{day:N}` is the day N days from today.
struct Demo {
    outline: Arc<Outline>,
    today: NaiveDate,
    ids: BTreeMap<String, String>,
    fold: Vec<String>,
}

impl Demo {
    fn day(&self, offset: i64) -> String {
        let day = if offset < 0 {
            self.today.checked_sub_days(Days::new(offset.unsigned_abs()))
        } else {
            self.today.checked_add_days(Days::new(offset.unsigned_abs()))
        };
        day.expect("a date in range").format("%Y-%m-%d").to_string()
    }

    /// `{day:N}` and `{name}` in `text`, filled in.
    fn fill(&self, text: &str) -> Result<String, FlowieError> {
        let mut out = String::new();
        let mut rest = text;
        while let Some(open) = rest.find('{') {
            let Some(close) = rest[open..].find('}') else { break };
            let name = &rest[open + 1..open + close];
            out.push_str(&rest[..open]);
            if let Some(offset) = name.strip_prefix("day:") {
                out.push_str(&self.day(offset.parse().expect("{day:N}")));
            } else {
                let id = self.ids.get(name).unwrap_or_else(|| panic!("no bullet {name} yet"));
                out.push_str(&self.outline.link(id.clone(), String::new())?);
            }
            rest = &rest[open + close + 1..];
        }
        out.push_str(rest);
        Ok(out)
    }

    /// Add `text`'s bullets under `parent` (None: Home), from child `start` on.
    fn build(&mut self, parent: Option<&str>, text: &str, start: u32) -> Result<(), FlowieError> {
        // (indent, id, children so far)
        let mut stack: Vec<(i32, Option<String>, u32)> = vec![(-1, parent.map(str::to_string), start)];
        for raw in text.lines().filter(|l| !l.trim().is_empty()) {
            let indent = (raw.len() - raw.trim_start().len()) as i32 / 2;
            let mut line = raw.trim_start().strip_prefix("- ").expect("a bullet line").to_string();
            let mut note = None;
            if let Some(at) = line.find(" ^note=") {
                note = Some(line[at + 7..].to_string());
                line.truncate(at);
            }
            let mut flags = Vec::new();
            while let Some(at) = line.rfind(" ^") {
                flags.push(line[at + 2..].to_string());
                line.truncate(at);
            }
            let done = line.starts_with("[x] ");
            if done {
                line.drain(..4);
            }
            while stack.last().expect("the parent").0 >= indent {
                stack.pop();
            }
            let top = stack.last_mut().expect("the parent");
            let (parent, index) = (top.1.clone(), top.2);
            top.2 += 1;
            let text = self.fill(&line)?;
            let id = self.outline.create(parent, index, text)?.focus.expect("the new bullet").id;
            if done {
                self.outline.set_completed(id.clone(), true)?;
            }
            if let Some(note) = note {
                self.outline.set_note(id.clone(), note)?;
            }
            for flag in flags {
                match flag.as_str() {
                    "star" => self.outline.set_starred(id.clone(), true)?,
                    "fold" => self.fold.push(id.clone()),
                    _ => {
                        let key = flag.strip_prefix("key=").unwrap_or_else(|| panic!("unknown flag ^{flag}"));
                        self.ids.insert(key.to_string(), id.clone());
                    }
                }
            }
            stack.push((indent, Some(id), 0));
        }
        Ok(())
    }
}

/// The text of one language.
struct Content {
    journal: &'static str,
    /// Earlier journal days (offset from today, text), written before the pages.
    earlier: [(i64, &'static str); 2],
    /// Today's first line, written before the pages.
    morning: &'static str,
    pages: &'static str,
    today: &'static str,
    /// Yesterday's second line, written last.
    yesterday: &'static str,
}

const EN: Content = Content {
    journal: "Journal",
    earlier: [(-3, "Started the Kyoto itinerary #idea"), (-1, "Reading group: *Make It Stick*, chapters 1–3")],
    morning: "Morning: outlined section 2 of the paper",
    pages: "
- Website redesign ^star ^key=web
  - Milestones ^key=milestones
    - [x] Pick colors and fonts #design
    - [x] Wireframes for the home page, signed off by @Maya
    - Rebuild the home page @Leo, due {day:3}
      - [x] Hero image and tagline
      - Mobile layout
    - Add analytics #dev
    - Launch on {day:12}: ==go / no-go call on Friday==
    - [x] Move the docs to the new theme
    - Test on phones and tablets #qa
      - [x] iPhone
      - iPad in landscape
    - Final copy from @Maya by {day:5}
    - Write the launch post ^note=Short: what changed, three screenshots, a link to the docs.
    - Redirect old blog links #dev
  - Meeting notes ^fold
    - {day:-1} weekly sync
    - {day:-8} kickoff
  - Open questions
    - Should old links redirect to the new pages? #to-discuss
    - The pricing page still has **no FAQ**
  - Ideas for later ^fold
    - Shared pages for small teams #idea
- Research notes: learning & memory ^star ^key=research
  - Spaced repetition beats cramming: **every study we read agrees** #idea
  - Review just before you forget; intervals grow about *2.5×* each time
  - [x] Summarize the Cepeda et al. meta-analysis
  - Draft section 2 and send it to @Maya by {day:1}
  - Does ==interleaving== help with math, or only with vocabulary? #idea
- Travel: Kyoto in spring ^key=travel
  - Book a ryokan near Gion ^note=Walking distance to Yasaka Shrine; ask for a garden view.
  - Check the cherry-blossom forecast in early March
  - Day trip to Nara #idea
  - Packing list ^fold
    - Rail pass
    - Camera and spare batteries
- Reading list ^star ^key=reading
  - [x] *Thinking, Fast and Slow* by Daniel Kahneman
  - [x] *Make It Stick* by Brown, Roediger & McDaniel
  - *How to Take Smart Notes* by Sönke Ahrens #idea
  - *Gödel, Escher, Bach* by Douglas Hofstadter
- Groceries
  - [x] Oat milk
  - Coffee beans
  - Lemons, basil, parmesan
- Ideas ^fold
  - A weekly review that pulls in every open to-do #idea
  - Turn reading highlights into flashcards #idea
",
    today: "
- Call with @Maya: she wants the draft by {day:1}
- Notes for {research}: ==retrieval practice== beats re-reading
- [x] Reply to the reviewers
- Order more notebooks
- Tomorrow
  - Lab meeting at 10:00 with @Leo
  - Plan the weekend hike #idea
- Booked the flights for {travel}
- Pick up film for the camera
",
    yesterday: "Coffee with @Leo about the retention model",
};

const ZH: Content = Content {
    journal: "日记",
    earlier: [(-3, "开始排京都行程 #想法"), (-1, "读书会：《认知天性》第 1–3 章")],
    morning: "上午：列好了论文第二节的提纲",
    pages: "
- 网站改版 ^star ^key=web
  - 里程碑 ^key=milestones
    - [x] 确定配色与字体 #设计
    - [x] 首页线框图，@小林 已确认
    - 首页重做 @老王，{day:3} 截止
      - [x] 头图与标语
      - 移动端适配
    - 接入统计 #开发
    - {day:12} 上线：==周五决定是否如期==
    - [x] 文档换成新主题
    - 在手机和平板上测试 #测试
      - [x] iPhone
      - iPad 横屏
    - {day:5} 前向 @小林 要最终文案
    - 写上线公告 ^note=简短：改了什么、三张截图、文档链接。
    - 旧博客链接做跳转 #开发
  - 会议记录 ^fold
    - {day:-1} 周会
    - {day:-8} 启动会
  - 待讨论
    - 旧链接要不要跳转到新页面？ #待讨论
    - 价格页还**缺常见问题**
  - 以后再做 ^fold
    - 小团队共享页面 #想法
- 研究笔记：学习与记忆 ^star ^key=research
  - 间隔重复胜过临时抱佛脚：**读过的研究都这么说** #想法
  - 在快要忘记时复习，每次间隔约增长 *2.5 倍*
  - [x] 总结 Cepeda 等人的元分析
  - 写完第二节初稿，{day:1} 前发给 @小林
  - ==交错练习==对数学有用，还是只对词汇有用？ #想法
- 旅行：春天去京都 ^key=travel
  - 订祇园附近的旅馆 ^note=走路能到八坂神社，要能看到庭院的房间。
  - 三月初查樱花预报
  - 去奈良一日游 #想法
  - 行李清单 ^fold
    - 铁路通票
    - 相机和备用电池
- 书单 ^star ^key=reading
  - [x] 《思考，快与慢》丹尼尔·卡尼曼
  - [x] 《认知天性》布朗、罗迪格、麦克丹尼尔
  - 《卡片笔记写作法》申克·阿伦斯 #想法
  - 《哥德尔、艾舍尔、巴赫》侯世达
- 采购
  - [x] 燕麦奶
  - 咖啡豆
  - 柠檬、罗勒、帕玛森奶酪
- 想法 ^fold
  - 每周回顾，自动汇总所有未完成的待办 #想法
  - 把读书摘录自动变成记忆卡片 #想法
",
    today: "
- 和 @小林 通话：她想在 {day:1} 前拿到初稿
- {research} 的笔记：==提取练习==比重读更有效
- [x] 回复审稿人
- 再买几本笔记本
- 明天
  - 10:00 和 @老王 开组会
  - 计划周末徒步 #想法
- 订好了去 {travel} 的机票
- 取冲洗好的胶卷
",
    yesterday: "和 @老王 喝咖啡，聊保持率模型",
};

fn main() -> Result<(), FlowieError> {
    let args: Vec<String> = std::env::args().collect();
    let [_, db, locale, today] = args.as_slice() else {
        eprintln!("usage: demo_outline <db path> <en-US|zh-Hans> <today YYYY-MM-DD>");
        std::process::exit(2);
    };
    let content = if locale.starts_with("zh") { &ZH } else { &EN };
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{db}{suffix}"));
    }
    let mut demo = Demo {
        outline: Outline::open(db.clone())?,
        today: NaiveDate::parse_from_str(today, "%Y-%m-%d").expect("today as YYYY-MM-DD"),
        ids: BTreeMap::new(),
        fold: Vec::new(),
    };
    let o = demo.outline.clone();

    // Earlier days first, so the journal sits above the pages on Home.
    for (offset, text) in content.earlier {
        o.journal_append(demo.day(offset), text.to_string())?;
    }
    o.journal_append(demo.day(0), content.morning.to_string())?;
    let today_id = o.journal_day(demo.day(0))?.expect("today's page");
    let journal = o.page(None)?.into_iter().find(|r| r.depth == 0).expect("the journal").id;
    o.set_text(journal.clone(), content.journal.to_string())?;
    demo.ids.insert("today".into(), today_id.clone());
    demo.ids.insert("journal".into(), journal.clone());

    demo.build(None, content.pages, 1)?;
    demo.build(Some(&today_id), content.today, 1)?;
    o.journal_append(demo.day(-1), content.yesterday.to_string())?;

    for id in &demo.fold {
        o.set_collapsed(id.clone(), true)?;
    }
    // The journal's earlier days stay folded, and so does the journal on Home: it has a tab.
    for row in o.page(Some(journal.clone()))? {
        if row.depth == 0 && row.id != today_id {
            o.set_collapsed(row.id, true)?;
        }
    }
    o.set_collapsed(journal, true)?;
    o.flush()?;

    let ids: Vec<String> = demo.ids.iter().map(|(k, v)| format!("{k:?}: {v:?}")).collect();
    println!("{{{}}}", ids.join(", "));
    Ok(())
}
