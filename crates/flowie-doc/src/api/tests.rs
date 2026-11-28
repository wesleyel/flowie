//! The facade's behavior, as iOS and the browser rely on it.

use flowie_core::{journal, BlockKind, NodeId, ROOT};

use super::*;

/// Add a bullet; its id.
fn add(outline: &mut Outline, parent: Option<&String>, index: u32, text: &str) -> String {
    outline.create(parent.map(String::as_str), index, text).unwrap().focus.unwrap().id
}

fn texts(outline: &mut Outline) -> Vec<(String, u16)> {
    outline.page(None).unwrap().into_iter().map(|r| (r.text, r.depth)).collect()
}

#[test]
fn edits_show_on_the_page_and_undo() {
    let mut outline = Outline::in_memory();
    let a = add(&mut outline, None, 0, "first");
    let b = add(&mut outline, Some(&a), 0, "child");
    outline.set_completed(&b, true).unwrap();
    let page = outline.page(None).unwrap();
    assert_eq!(page.iter().map(|r| (r.text.as_str(), r.depth, r.completed)).collect::<Vec<_>>(), [("first", 0, false), ("child", 1, true)]);
    assert!(page[0].has_children);
    assert_eq!(outline.page(Some(a.as_str())).unwrap().len(), 1);
    outline.delete(&b).unwrap();
    assert_eq!(outline.page(None).unwrap().len(), 1);
    assert!(outline.undo().structure_changed);
    assert_eq!(outline.page(None).unwrap().len(), 2);
    assert!(matches!(outline.set_text("nope", "x"), Err(Error::NoSuchBullet { .. })));

    let c = add(&mut outline, None, 1, "second");
    assert!(outline.indent(&c).unwrap().structure_changed);
    assert_eq!(
        outline.page(None).unwrap().iter().map(|r| (r.text.as_str(), r.depth)).collect::<Vec<_>>(),
        [("first", 0), ("child", 1), ("second", 1)]
    );
    assert!(outline.outdent(&c, Some(a.as_str())).unwrap().is_empty(), "not out of the page shown");
    assert!(outline.outdent(&c, None).unwrap().structure_changed);
    assert_eq!(outline.page(None).unwrap()[2].depth, 0);
    assert!(outline.outdent(&c, None).unwrap().is_empty());
    assert!(outline.undo().structure_changed);
    assert_eq!(outline.page(None).unwrap()[2].depth, 1);
}

#[test]
fn backlinks_list_the_bullets_linking_to_a_page_then_its_mirrors() {
    let mut outline = Outline::in_memory();
    let page = add(&mut outline, None, 0, "Page");
    let project = add(&mut outline, None, 1, "Project");
    let later = add(&mut outline, Some(&project), 0, &format!("see [it](flowie:{page}) **now**"));
    let first = add(&mut outline, None, 0, &format!("[Page](flowie:{page})"));
    assert!(outline.backlinks(&project).unwrap().is_empty());
    let mirror = {
        let (p, src) = (parse(&project).unwrap(), parse(&page).unwrap());
        outline.ws.tree.create_mirror(src, p, 1).unwrap().to_string()
    };
    let links = outline.backlinks(&page).unwrap();
    // In outline order, links before mirrors; the text as stored, to render.
    assert_eq!(
        links.iter().map(|b| (b.id.as_str(), b.mirror)).collect::<Vec<_>>(),
        [(first.as_str(), false), (later.as_str(), false), (mirror.as_str(), true)]
    );
    assert_eq!(links[1].text, format!("see [it](flowie:{page}) **now**"));
    assert_eq!(links[1].path, [Crumb { id: project.clone(), text: "Project".into() }]);
    assert!(links[0].path.is_empty());
    // A link opens its own bullet; a mirror, the page it is on.
    assert_eq!(links[1].target.as_deref(), Some(later.as_str()));
    assert_eq!(links[2].target.as_deref(), Some(project.as_str()));
    assert_eq!(links[2].text, "Page");
    // From the mirror's page, the same list; gone with the link.
    assert_eq!(outline.backlinks(&mirror).unwrap().len(), 3);
    outline.delete(&first).unwrap();
    assert_eq!(outline.backlinks(&page).unwrap().len(), 2);
    assert!(outline.backlinks("nope").is_err());
}

#[test]
fn format_commands_write_the_desktops_markup_in_utf16() {
    let f = |text: &str, start, end, format| {
        let out = toggle_format(text, start, end, format);
        (out.text, out.start, out.end)
    };
    // "加粗" is two UTF-16 units, as is the emoji before it.
    assert_eq!(f("😀加粗 x", 2, 4, TextFormat::Bold), ("😀**加粗** x".into(), 4, 6));
    assert_eq!(f("😀**加粗** x", 4, 6, TextFormat::Bold), ("😀加粗 x".into(), 2, 4));
    assert_eq!(f("a b", 2, 3, TextFormat::Italic).0, "a *b*");
    assert_eq!(f("a b", 2, 3, TextFormat::Strikethrough).0, "a ~~b~~");
    assert_eq!(f("a b", 2, 3, TextFormat::Code).0, "a `b`");
    assert_eq!(f("ab", 1, 1, TextFormat::Highlight), ("a====b".into(), 3, 3));
    assert_eq!(f("a b", 2, 3, TextFormat::Link), ("a [b](https://)".into(), 6, 14));
    assert_eq!(f("", 0, 0, TextFormat::Link), ("[link](https://)".into(), 7, 15));
}

#[test]
fn return_and_backspace_split_and_merge_rows() {
    let mut outline = Outline::in_memory();
    let a = add(&mut outline, None, 0, "hello world");
    let b = add(&mut outline, None, 1, "b");
    add(&mut outline, Some(&b), 0, "child");

    let out = outline.enter(&a, 5, 5, None).unwrap();
    assert!(out.structure_changed);
    let focus = out.focus.unwrap();
    assert_eq!((focus.start, focus.end), (0, 0));
    let rows = outline.page(None).unwrap();
    assert_eq!((rows[1].id.as_str(), rows[1].text.as_str()), (focus.id.as_str(), " world"));
    // Backspace at the start of the new row joins it back, the caret where they meet.
    let back = outline.backspace_at_start(&focus.id, None).unwrap().focus.unwrap();
    assert_eq!((back.id.as_str(), back.start, back.end), (a.as_str(), 5, 5));
    assert_eq!(texts(&mut outline)[0].0, "hello world");
    // A row with children joins a leaf above, children and all.
    outline.backspace_at_start(&b, None).unwrap().focus.unwrap();
    assert_eq!(texts(&mut outline), [("hello worldb".to_string(), 0), ("child".to_string(), 1)]);
    assert_eq!(outline.backspace_at_start(&a, None).unwrap(), EditOutcome::default(), "the first row");
    // Each key is one undo step.
    assert!(outline.undo().structure_changed);
    assert!(outline.undo().structure_changed);
    assert_eq!(texts(&mut outline)[..2], [("hello".to_string(), 0), (" world".to_string(), 0)]);
    // Undo puts the caret back where it was before Return.
    let undone = outline.undo();
    assert_eq!(undone.focus, Some(FocusView { id: a.clone(), start: 5, end: 5 }));
    assert_eq!(texts(&mut outline)[0].0, "hello world");
}

#[test]
fn rows_move_up_and_down_within_the_page() {
    let mut outline = Outline::in_memory();
    let a = add(&mut outline, None, 0, "a");
    let b = add(&mut outline, None, 1, "b");
    add(&mut outline, Some(&a), 0, "a1");
    let out = outline.move_up(&b, None).unwrap();
    assert!(out.structure_changed);
    assert_eq!(texts(&mut outline), [("b".into(), 0), ("a".into(), 0), ("a1".into(), 1)]);
    assert!(outline.move_up(&b, None).unwrap().is_empty(), "already first");
    assert!(outline.move_down(&b, None).unwrap().structure_changed);
    assert_eq!(texts(&mut outline), [("a".into(), 0), ("a1".into(), 1), ("b".into(), 0)]);
    // The first child goes into the bullet before its parent, except out of the page.
    let b1 = add(&mut outline, Some(&b), 0, "b1");
    assert!(outline.move_up(&b1, Some(b.as_str())).unwrap().is_empty(), "not out of the page shown");
    outline.move_up(&b1, None).unwrap();
    assert_eq!(texts(&mut outline), [("a".into(), 0), ("a1".into(), 1), ("b1".into(), 1), ("b".into(), 0)]);
    assert!(outline.move_down(&b1, None).unwrap().structure_changed);
    assert_eq!(texts(&mut outline)[2..], [("b".into(), 0), ("b1".into(), 1)]);
    // Each move is an undo step.
    outline.undo();
    assert_eq!(texts(&mut outline)[2], ("b1".into(), 1));
    assert!(matches!(outline.move_up("nope", None), Err(Error::NoSuchBullet { .. })));
}

#[test]
fn offsets_cross_in_utf16() {
    let mut outline = Outline::in_memory();
    // 日记: 3 bytes and 1 unit each; 😀: 4 bytes, 2 units.
    let a = add(&mut outline, None, 0, "日记😀x");
    let out = outline.create(None, 1, "日😀").unwrap();
    assert_eq!(out.focus.map(|f| (f.start, f.end)), Some((3, 3)), "the end of the text, in UTF-16");
    // Return after 记 (unit 2), and with 记😀 (units 1..4) selected.
    let focus = outline.enter(&a, 2, 2, None).unwrap().focus.unwrap();
    assert_eq!(texts(&mut outline)[..2], [("日记".to_string(), 0), ("😀x".to_string(), 0)]);
    let back = outline.backspace_at_start(&focus.id, None).unwrap().focus.unwrap();
    assert_eq!((back.start, back.end), (2, 2), "after 日记, in UTF-16");
    outline.enter(&a, 1, 4, None).unwrap();
    assert_eq!(texts(&mut outline)[..2], [("日".to_string(), 0), ("x".to_string(), 0)]);
    // A caret between the halves of 😀 moves past it.
    let b = add(&mut outline, None, 0, "😀😀");
    outline.enter(&b, 1, 1, None).unwrap();
    assert_eq!(texts(&mut outline)[..2], [("😀".to_string(), 0), ("😀".to_string(), 0)]);
}

#[test]
fn typing_changes_only_its_row() {
    let mut outline = Outline::in_memory();
    let a = add(&mut outline, None, 0, "a");
    let b = add(&mut outline, Some(&a), 0, "b");
    let out = outline.set_text(&b, "b, typed 中文 ✓").unwrap();
    assert_eq!(out, EditOutcome { changed_ids: vec![b.clone()], structure_changed: false, focus: None });
    // The changed row reads the same as on the page.
    let page = outline.page(None).unwrap();
    assert_eq!(outline.rows(&out.changed_ids, None).unwrap(), [page[1].clone()]);
    assert_eq!(outline.rows(&[b.clone(), a.clone()], Some(a.as_str())).unwrap()[..], [RowView { depth: 0, ..page[1].clone() }]);
    assert!(outline.rows(&["nope".into()], None).unwrap().is_empty());
    let out = outline.set_completed(&b, true).unwrap();
    assert_eq!((out.changed_ids.as_slice(), out.structure_changed), (std::slice::from_ref(&b), false));
    let out = outline.set_note(&a, "note").unwrap();
    assert_eq!((out.changed_ids.as_slice(), out.structure_changed), (std::slice::from_ref(&a), false));
    // Undoing typing changes the row back.
    let out = outline.undo();
    assert_eq!((out.changed_ids, out.structure_changed), (vec![a.clone()], false));
    // A row hidden under a collapsed bullet isn't on the page.
    let id = a.parse().unwrap();
    outline.ws.tree.set_collapsed(id, true);
    assert!(outline.rows(std::slice::from_ref(&b), None).unwrap().is_empty());
    // Deleting is structural.
    let out = outline.delete(&b).unwrap();
    assert!(out.structure_changed);
    assert!(outline.redo().is_empty(), "nothing to redo after an edit");
}

#[test]
fn journal_days_are_found_and_opened_like_on_the_desktop() {
    let mut outline = Outline::in_memory();
    assert_eq!(outline.journal_day("2026-11-27").unwrap(), None, "nothing is made by looking");
    assert!(outline.page(None).unwrap().is_empty());
    let out = outline.open_day("2026-11-27").unwrap();
    assert!(out.structure_changed);
    let day = outline.journal_day("2026-11-27").unwrap().unwrap();
    assert_eq!(day, journal::day_id("2026-11-27").to_string(), "the same id on every device");
    let rows = outline.page(Some(day.as_str())).unwrap();
    assert_eq!(rows.len(), 1, "a blank first line");
    assert_eq!(out.focus, Some(FocusView { id: rows[0].id.clone(), start: 0, end: 0 }));
    assert_eq!(rows[0].id, journal::day_blank_id("2026-11-27").to_string());
    // Opening it again makes nothing, and the caret goes to the end of its first row.
    outline.set_text(&rows[0].id, "日记").unwrap();
    let again = outline.open_day("2026-11-27").unwrap();
    assert!(!again.structure_changed && again.changed_ids.is_empty());
    assert_eq!(again.focus.map(|f| f.start), Some(2));
    assert_eq!(outline.page(None).unwrap().iter().filter(|r| r.text == "2026-11-27").count(), 1, "one page for the day");
    // Days sort newest first under the journal.
    outline.open_day("2026-11-28").unwrap();
    let journal = journal::journal_root_id().to_string();
    let days: Vec<String> = outline.page(Some(journal.as_str())).unwrap().into_iter().filter(|r| r.depth == 0).map(|r| r.text).collect();
    assert_eq!(days, ["2026-11-28", "2026-11-27"]);
    assert_eq!(outline.journal_date(&day).as_deref(), Some("2026-11-27"), "a day opens as its day");
    assert_eq!(outline.journal_date(&rows[0].id), None);
    assert_eq!(outline.journal_date(&journal::journal_root_id().to_string()), None);
    assert!(matches!(outline.open_day("2026-11-27; drop"), Err(Error::NotADate { .. })));
    assert!(matches!(outline.journal_day("tomorrow"), Err(Error::NotADate { .. })));
}

#[test]
fn journal_capture_and_append_use_the_desktops_day() {
    let mut outline = Outline::in_memory();
    let date = "2026-11-27";
    let page = Some(journal::day_id(date).to_string());
    assert!(matches!(outline.journal_append("yesterday", "oops"), Err(Error::NotADate { .. })));
    assert!(matches!(outline.journal_capture("2026-13-01"), Err(Error::NotADate { .. })));

    // Capturing opens the day with the desktop's blank first line, again and again.
    let out = outline.journal_capture(date).unwrap();
    let blank = journal::day_blank_id(date).to_string();
    assert_eq!(out.focus, Some(FocusView { id: blank.clone(), start: 0, end: 0 }));
    assert!(out.structure_changed);
    assert_eq!(outline.journal_capture(date).unwrap().focus.map(|f| f.id), Some(blank.clone()));
    assert_eq!(outline.page(page.as_deref()).unwrap().len(), 1);

    // Appending replaces that blank line; the focus is in UTF-16.
    let raw = " **日😀** \nsecond";
    let out = outline.journal_append(date, raw).unwrap();
    let rows = outline.page(page.as_deref()).unwrap();
    assert_eq!(rows.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(), ["**日😀**", "second"]);
    assert_eq!(out.focus, Some(FocusView { id: rows[1].id.clone(), start: 6, end: 6 }));
    assert_eq!(outline.journal_day(date).unwrap(), page);
    let blank_out = outline.journal_append(date, " \n ").unwrap();
    assert!(blank_out.focus.is_none() && blank_out.changed_ids.is_empty());

    outline.undo();
    let rows = outline.page(page.as_deref()).unwrap();
    assert_eq!(rows.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(), [""], "one step");
    assert_eq!(rows[0].id, blank);
}

#[test]
fn folding_hides_children_and_undoes() {
    let mut outline = Outline::in_memory();
    let a = add(&mut outline, None, 0, "a");
    let b = add(&mut outline, Some(&a), 0, "b");
    assert!(outline.set_collapsed(&b, true).unwrap().is_empty(), "nothing to fold");
    let out = outline.set_collapsed(&a, true).unwrap();
    assert!(out.structure_changed);
    let page = outline.page(None).unwrap();
    assert_eq!(page.len(), 1);
    assert!(page[0].collapsed && page[0].has_children);
    assert!(outline.set_collapsed(&a, true).unwrap().is_empty(), "already folded");
    // A folded bullet's page still shows its children.
    assert_eq!(outline.page(Some(a.as_str())).unwrap().len(), 1);
    outline.set_collapsed(&a, false).unwrap();
    assert_eq!(outline.page(None).unwrap().len(), 2);
    assert!(outline.undo().structure_changed);
    assert_eq!(outline.page(None).unwrap().len(), 1, "undo folds it again");
}

#[test]
fn expanding_to_a_level_folds_below_it_and_undoes() {
    let mut outline = Outline::in_memory();
    let a = add(&mut outline, None, 0, "a");
    let b = add(&mut outline, Some(&a), 0, "b");
    add(&mut outline, Some(&b), 0, "c");
    add(&mut outline, None, 1, "d");
    let names = |o: &mut Outline| o.page(None).unwrap().into_iter().map(|r| r.text).collect::<Vec<_>>();
    let out = outline.expand_to_level(None, 1).unwrap();
    assert!(out.structure_changed);
    assert_eq!(names(&mut outline), ["a", "d"], "collapse all");
    outline.expand_to_level(None, 2).unwrap();
    assert_eq!(names(&mut outline), ["a", "b", "d"]);
    assert!(outline.page(None).unwrap()[1].collapsed);
    outline.expand_to_level(None, u32::MAX).unwrap();
    assert_eq!(names(&mut outline), ["a", "b", "c", "d"]);
    assert!(outline.expand_to_level(None, u32::MAX).unwrap().is_empty(), "already expanded");
    assert!(outline.undo().structure_changed);
    assert_eq!(names(&mut outline), ["a", "b", "d"], "one undo step");
    // On a bullet's page, levels count from it.
    outline.expand_to_level(Some(a.as_str()), 1).unwrap();
    assert_eq!(outline.page(Some(a.as_str())).unwrap().len(), 1);
    assert!(outline.expand_to_level(Some("nope"), 1).is_err());
}

#[test]
fn stars_list_and_name_their_bullets() {
    let mut outline = Outline::in_memory();
    let a = add(&mut outline, None, 0, "**Projects**");
    let b = add(&mut outline, Some(&a), 0, "Flowie");
    assert!(outline.starred().is_empty());
    outline.set_starred(&b, true).unwrap();
    outline.set_starred(&a, true).unwrap();
    let stars = outline.starred();
    assert_eq!(stars.iter().map(|s| s.text.as_str()).collect::<Vec<_>>(), ["Flowie", "Projects"], "in starring order, plain");
    assert_eq!(stars[0].path, [Crumb { id: a.clone(), text: "Projects".into() }]);
    assert!(stars[0].starred && stars[1].has_children);
    outline.set_starred(&b, false).unwrap();
    assert_eq!(outline.starred().len(), 1);
    assert!(!outline.bullet(&b).unwrap().starred);
    // Stars aren't undo steps.
    assert!(outline.undo().structure_changed, "undo undoes the last edit, not the star");
    assert_eq!(outline.page(None).unwrap().len(), 1);
    assert!(outline.bullet(&a).unwrap().starred);
    assert!(matches!(outline.set_starred("nope", true), Err(Error::NoSuchBullet { .. })));
}

#[test]
fn index_pages_are_parents_stars_and_days() {
    let mut outline = Outline::in_memory();
    let a = add(&mut outline, None, 0, "**Projects**");
    let b = add(&mut outline, Some(&a), 0, "Flowie");
    let c = add(&mut outline, Some(&b), 0, "ship it");
    add(&mut outline, None, 1, "lone");
    let star = add(&mut outline, None, 2, "starred leaf");
    outline.set_starred(&star, true).unwrap();
    outline.open_day("2026-11-27").unwrap();
    let pages = outline.index_pages();
    let names: Vec<&str> = pages.iter().map(|p| p.title.as_str()).collect();
    assert_eq!(names, ["Projects", "Flowie", "starred leaf", "Journal", "2026-11-27"], "not leaves, in outline order");
    assert_eq!(pages[1].path, ["Projects"]);
    assert_eq!(pages[1].content, "ship it");
    assert!(pages[2].starred);
    assert_eq!(pages[4].day.as_deref(), Some("2026-11-27"));
    assert_eq!(pages[4].path, ["Journal"]);
    // The digest follows what is indexed: a child's text, the path, a star.
    let before = pages[1].digest;
    assert_eq!(outline.index_pages()[1].digest, before, "stable");
    outline.set_text(&c, "shipped").unwrap();
    let after = outline.index_pages();
    assert_ne!(after[1].digest, before);
    assert_eq!(after[0].digest, pages[0].digest, "the parent's own children didn't change");
    outline.set_text(&a, "Work").unwrap();
    assert_ne!(outline.index_pages()[1].digest, after[1].digest, "the path changed");
    // Deleted pages leave the list.
    outline.delete(&b).unwrap();
    assert_eq!(outline.index_pages().iter().map(|p| p.title.as_str()).collect::<Vec<_>>(), ["starred leaf", "Journal", "2026-11-27"]);
}

#[test]
fn saved_searches_list_the_pref() {
    let mut outline = Outline::in_memory();
    assert!(outline.saved_searches().is_empty());
    outline.ws.set_pref("saved_searches", "is:todo\n\n#work");
    assert_eq!(outline.saved_searches(), ["is:todo", "#work"]);
}

#[test]
fn search_takes_the_desktop_filters() {
    let mut outline = Outline::in_memory();
    let a = add(&mut outline, None, 0, "Errands");
    let milk = add(&mut outline, Some(&a), 0, "buy milk #shop");
    let bread = add(&mut outline, Some(&a), 1, "buy bread #shop");
    outline.set_completed(&bread, true).unwrap();
    outline.set_note(&milk, "oat").unwrap();
    let ids = |hits: Vec<BulletView>| hits.into_iter().map(|h| h.id).collect::<Vec<_>>();
    assert_eq!(ids(outline.search("#shop is:open", 10)), std::slice::from_ref(&milk));
    assert_eq!(ids(outline.search("oat", 10)), std::slice::from_ref(&milk), "notes are searched");
    assert_eq!(ids(outline.search("#shop -milk", 10)), std::slice::from_ref(&bread));
    assert_eq!(outline.search("#shop", 1).len(), 1, "at most the limit");
    assert!(outline.search("  ", 10).is_empty());
    let hit = &outline.search("has:note", 10)[0];
    assert_eq!((hit.text.as_str(), hit.path[0].text.as_str()), ("buy milk #shop", "Errands"));
}

#[test]
fn drop_place_and_drop_bullet_move_a_bullet_between_pages() {
    let mut outline = Outline::in_memory();
    let a = add(&mut outline, None, 0, "a");
    let b = add(&mut outline, Some(&a), 0, "b");
    let c = add(&mut outline, None, 1, "c");
    // c dropped on a's page, below b, one level in: under b.
    let place = outline.drop_place(Some(a.as_str()), Some(c.as_str()), Some(b.as_str()), 0, None, 1).unwrap().unwrap();
    assert_eq!(place, DropPlace { parent: Some(b.clone()), index: 0, depth: 1 });
    assert!(outline.drop_bullet(&c, &place).unwrap().structure_changed);
    assert_eq!(
        outline.page(Some(a.as_str())).unwrap().iter().map(|r| (r.text.as_str(), r.depth)).collect::<Vec<_>>(),
        [("b", 0), ("c", 1)]
    );
    outline.undo();
    assert_eq!(outline.page(None).unwrap().iter().map(|r| r.text.as_str()).collect::<Vec<_>>(), ["a", "b", "c"]);
    // At the top of Home: Home's first child. Not under itself.
    let top = outline.drop_place(None, Some(c.as_str()), None, 0, Some(0), 3).unwrap().unwrap();
    assert_eq!(top, DropPlace { parent: None, index: 0, depth: 0 });
    assert_eq!(outline.drop_place(Some(a.as_str()), Some(a.as_str()), Some(b.as_str()), 0, None, 1).unwrap(), None);
}

#[test]
fn move_to_puts_a_bullet_last_under_another() {
    let mut outline = Outline::in_memory();
    let inbox = add(&mut outline, None, 0, "Inbox");
    let task = add(&mut outline, Some(&inbox), 0, "a task");
    let project = add(&mut outline, None, 1, "Project");
    add(&mut outline, Some(&project), 0, "first step");
    outline.set_collapsed(&project, true).unwrap();
    // The picker leaves out the bullet and what is under it.
    let found: Vec<String> = outline.quick_find("", std::slice::from_ref(&inbox), 10).into_iter().map(|b| b.text).collect();
    assert_eq!(found.len(), 2);
    assert!(!found.contains(&"Inbox".to_string()) && !found.contains(&"a task".to_string()));
    assert_eq!(outline.quick_find("proj", &[], 10)[0].id, project);
    let out = outline.move_to(&task, Some(project.as_str())).unwrap();
    assert!(out.structure_changed);
    assert_eq!(
        texts(&mut outline),
        [("Inbox".into(), 0), ("Project".into(), 0), ("first step".into(), 1), ("a task".into(), 1)],
        "last, unfolded"
    );
    assert!(outline.move_to(&project, Some(task.as_str())).unwrap().is_empty(), "not under itself");
    outline.move_to(&task, None).unwrap();
    assert_eq!(texts(&mut outline).last(), Some(&("a task".to_string(), 0)), "to the end of Home");
    outline.undo();
    outline.undo();
    assert_eq!(texts(&mut outline)[1], ("a task".into(), 1), "each move an undo step");
    assert!(matches!(outline.move_to(&task, Some("nope")), Err(Error::NoSuchBullet { .. })));
}

#[test]
fn completed_rows_hide_and_the_setting_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("outline.sqlite").to_string_lossy().into_owned();
    let mut outline = Outline::open(db.as_ref()).unwrap();
    let a = add(&mut outline, None, 0, "a");
    let b = add(&mut outline, Some(&a), 0, "b");
    let c = add(&mut outline, Some(&b), 0, "c");
    add(&mut outline, None, 1, "d");
    assert!(!outline.hide_completed());
    outline.set_hide_completed(true);
    let out = outline.set_completed(&b, true).unwrap();
    assert!(out.structure_changed, "the row goes from the page");
    assert_eq!(texts(&mut outline), [("a".into(), 0), ("d".into(), 0)], "with what is under it");
    assert!(outline.rows(&[b.clone(), c.clone()], None).unwrap().is_empty());
    // Typing never changes what shows.
    assert!(!outline.set_text(&a, "a!").unwrap().structure_changed);
    // Undoing the completion brings it back as rows coming.
    outline.undo();
    assert!(outline.undo().structure_changed);
    assert_eq!(outline.page(None).unwrap().len(), 4);
    // Return at the start of the row after a hidden one joins the row shown above.
    outline.set_completed(&b, true).unwrap();
    let d = outline.page(None).unwrap()[1].id.clone();
    let joined = outline.backspace_at_start(&d, None).unwrap().focus.unwrap();
    assert_eq!(joined.id, a);
    outline.flush().unwrap();
    drop(outline);
    let mut outline = Outline::open(db.as_ref()).unwrap();
    assert!(outline.hide_completed(), "kept for this device");
    outline.set_hide_completed(false);
    assert_eq!(outline.page(None).unwrap().len(), 3);
}

#[test]
fn deleted_bullets_list_and_restore() {
    let mut outline = Outline::in_memory();
    let a = add(&mut outline, None, 0, "keep");
    let b = add(&mut outline, None, 1, "**gone** for now");
    add(&mut outline, Some(&b), 0, "child");
    outline.delete(&b).unwrap();
    let trash = outline.trashed();
    assert_eq!(trash.len(), 1);
    assert_eq!((trash[0].id.as_str(), trash[0].text.as_str(), trash[0].count), (b.as_str(), "gone for now", 2));
    assert!(trash[0].deleted_at > 0);
    assert!(matches!(outline.bullet(&b), Err(Error::NoSuchBullet { .. })));
    let out = outline.restore(&b).unwrap();
    assert!(out.structure_changed);
    assert_eq!(out.focus, Some(FocusView { id: b.clone(), start: 16, end: 16 }));
    assert_eq!(texts(&mut outline), [("keep".into(), 0), ("**gone** for now".into(), 0), ("child".into(), 1)]);
    assert!(outline.trashed().is_empty());
    assert!(matches!(outline.restore(&a), Err(Error::NoSuchBullet { .. })), "not in the Trash");
    assert!(outline.undo().structure_changed);
    assert_eq!(outline.trashed().len(), 1, "undo deletes it again");
}

#[test]
fn links_and_paths_name_bullets() {
    let mut outline = Outline::in_memory();
    let a = add(&mut outline, None, 0, "Work [2026]");
    let b = add(&mut outline, Some(&a), 0, "");
    assert_eq!(outline.link(&a, "Untitled").unwrap(), format!("[Work 2026](flowie:{a})"));
    assert_eq!(outline.link(&b, "无标题").unwrap(), format!("[无标题](flowie:{b})"));
    let info = outline.bullet(&b).unwrap();
    assert_eq!(info.path, [Crumb { id: a.clone(), text: "Work [2026]".into() }]);
    assert!(outline.bullet(&a).unwrap().path.is_empty(), "Home isn't on the path");
}

#[test]
fn markup_runs_count_utf16() {
    let id = NodeId::random();
    // 日: 3 bytes, 1 unit; 😀: 4 bytes, 2 units.
    let text =
        format!("日😀 **粗** *斜* ~~删~~ [链接](flowie:{id}) #标签 2026-11-27 $x^2$ {{red}}红{{/}} ==亮== `c` https://example.com/a");
    let runs = markup(&text);
    let units: Vec<u16> = text.encode_utf16().collect();
    let shown = |r: &MarkupRun| String::from_utf16(&units[r.start as usize..r.end as usize]).unwrap();
    // The runs cover the text in order.
    assert_eq!(runs.first().map(|r| r.start), Some(0));
    assert_eq!(runs.last().map(|r| r.end as usize), Some(units.len()));
    assert!(runs.windows(2).all(|w| w[0].end == w[1].start));
    let find = |kind: fn(&RunKind) -> bool| runs.iter().find(|r| kind(&r.kind)).unwrap();
    let bold = runs.iter().find(|r| r.bold && r.kind == RunKind::Text).unwrap();
    assert_eq!((bold.start, shown(bold).as_str()), (6, "粗"), "after 日😀 and a space and **");
    let link = find(|k| matches!(k, RunKind::Link { bullet: Some(_), .. }));
    assert_eq!(shown(link), "链接");
    assert_eq!(link.kind, RunKind::Link { url: format!("flowie:{id}"), bullet: Some(id.to_string()) });
    assert_eq!(shown(find(|k| *k == RunKind::Tag)), "#标签");
    assert_eq!(shown(find(|k| *k == RunKind::Date)), "2026-11-27");
    assert_eq!(shown(find(|k| matches!(k, RunKind::Math { display: false }))), "$x^2$");
    assert_eq!(shown(find(|k| *k == RunKind::Code)), "c");
    let red = runs.iter().find(|r| r.color == Some(0)).unwrap();
    assert_eq!(shown(red), "红");
    assert!(runs.iter().any(|r| r.italic && shown(r) == "斜"));
    assert!(runs.iter().any(|r| r.strike && shown(r) == "删"));
    assert!(runs.iter().any(|r| r.highlight && shown(r) == "亮"));
    let web = runs.last().unwrap();
    assert_eq!(web.kind, RunKind::Link { url: "https://example.com/a".into(), bullet: None });
    assert!(markup("").is_empty());
    assert_eq!(markup("[x](flowie:nope)")[1].kind, RunKind::Link { url: "flowie:nope".into(), bullet: None });
}

#[test]
fn image_rows_say_where_their_file_is_and_its_size() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("outline.sqlite");
    let mut outline = Outline::open(&db).unwrap();
    // A 300×200 PNG header, stored as the desktop stores images.
    let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
    png.extend(300u32.to_be_bytes());
    png.extend(200u32.to_be_bytes());
    png.extend([8, 6, 0, 0, 0, 0, 0, 0, 0]);
    let (shown, missing, plain) = {
        let ws = &mut outline.ws;
        let info = ws.image_dir.as_ref().unwrap().put(&png).unwrap();
        let name = info.name.clone();
        ws.record_blob(info);
        let mut ids = Vec::new();
        for (image, caption) in [(name, "a caption"), (format!("{}.png", "ab".repeat(32)), "")] {
            let id = ws.tree.create_at(ROOT, ids.len(), caption);
            let mut node = ws.tree.node(id).clone();
            node.kind = BlockKind::Image;
            node.image = image;
            ws.tree.put(node);
            ids.push(id);
        }
        let plain = ws.tree.create_at(ROOT, 2, "no picture");
        ws.commit_with_imports(None, None, None);
        (ids[0].to_string(), ids[1].to_string(), plain.to_string())
    };
    let page = outline.page(None).unwrap();
    let image = page[0].image.clone().unwrap();
    assert_eq!((page[0].kind.as_str(), page[0].text.as_str()), ("image", "a caption"));
    assert_eq!((image.width, image.height), (300, 200), "from the blobs table");
    let path = std::path::PathBuf::from(image.path.unwrap());
    assert!(path.starts_with(dir.path().join("outline.sqlite.images")) && path.exists());
    // Synced without its file: where it would be, and no size.
    let missing_image = page[1].image.clone().unwrap();
    assert_eq!((missing_image.width, missing_image.height), (0, 0));
    assert!(!std::path::Path::new(&missing_image.path.unwrap()).exists());
    assert_eq!(page[2].image, None);
    assert_eq!(outline.rows(std::slice::from_ref(&shown), None).unwrap()[0].image, page[0].image, "rows() says the same");
    // Sizes are kept with the database.
    outline.flush().unwrap();
    drop(outline);
    let mut outline = Outline::open(&db).unwrap();
    let again = outline.rows(&[shown, missing, plain], None).unwrap();
    assert_eq!(again[0].image.as_ref().map(|i| (i.width, i.height)), Some((300, 200)));
    assert!(again[1].image.is_some() && again[2].image.is_none());
}

#[test]
fn a_packed_page_reads_back_as_the_page() {
    struct Reader<'a>(&'a [u8]);
    impl Reader<'_> {
        fn take(&mut self, n: usize) -> &[u8] {
            let (head, rest) = self.0.split_at(n);
            self.0 = rest;
            head
        }
        fn u32(&mut self) -> u32 {
            u32::from_le_bytes(self.take(4).try_into().unwrap())
        }
        fn string(&mut self) -> String {
            let len = self.u32() as usize;
            String::from_utf8(self.take(len).to_vec()).unwrap()
        }
    }
    fn unpack(bytes: &[u8]) -> Vec<RowView> {
        let mut r = Reader(bytes);
        let rows = (0..r.u32())
            .map(|_| {
                let depth = u16::from_le_bytes(r.take(2).try_into().unwrap());
                let flags = r.take(1)[0];
                let [id, text, note, kind] = std::array::from_fn(|_| r.string());
                let image =
                    (flags & 8 != 0).then(|| ImageView { width: r.u32(), height: r.u32(), path: (flags & 16 != 0).then(|| r.string()) });
                RowView {
                    id,
                    depth,
                    text,
                    note,
                    kind,
                    completed: flags & 1 != 0,
                    collapsed: flags & 2 != 0,
                    has_children: flags & 4 != 0,
                    image,
                }
            })
            .collect();
        assert!(r.0.is_empty(), "nothing left over");
        rows
    }
    let mut outline = Outline::in_memory();
    let parent = add(&mut outline, None, 0, "parent, **bold** 中文 ✓");
    add(&mut outline, Some(&parent), 0, "child");
    outline.set_note(&parent, "a note").unwrap();
    let done = add(&mut outline, None, 1, "");
    outline.set_completed(&done, true).unwrap();
    let page = outline.page(None).unwrap();
    assert_eq!(unpack(&outline.page_packed(None).unwrap()), page);
    let image = |path: Option<&str>| ImageView { path: path.map(str::to_string), width: 300, height: 200 };
    let mut rows = page;
    rows[0].collapsed = true;
    rows[1].image = Some(image(Some("/images/ab/cd.png")));
    rows[2].image = Some(image(None));
    assert_eq!(unpack(&pack_rows(&rows)), rows);
    assert_eq!(unpack(&pack_rows(&[])), vec![]);
}

#[test]
fn rows_inside_a_mirror_show_and_edit_their_source() {
    let mut outline = Outline::in_memory();
    let source = add(&mut outline, None, 0, "source");
    let child = add(&mut outline, Some(&source), 0, "child");
    let mirror = {
        let source = source.parse().unwrap();
        let mirror = outline.ws.tree.create_mirror(source, ROOT, 1).unwrap();
        outline.ws.commit_with_imports(None, None, None);
        mirror.to_string()
    };
    let virtual_child = flowie_core::tree::virtual_id(mirror.parse().unwrap(), child.parse().unwrap()).to_string();
    assert_eq!(outline.rows(std::slice::from_ref(&virtual_child), None).unwrap()[0].text, "child");
    let page = outline.page(None).unwrap();
    assert_eq!(page.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(), ["source", "child", "source", "child"]);
    assert!(page[2].has_children && outline.bullet(&mirror).unwrap().has_children);
    assert_eq!(page[2].id, mirror);
    let inside = page[3].id.clone();
    assert_ne!(inside, child, "a virtual id");
    outline.set_text(&inside, "child, edited in the mirror").unwrap();
    outline.set_note(&mirror, "a note through the mirror").unwrap();
    let page = outline.page(None).unwrap();
    assert_eq!(page[1].text, "child, edited in the mirror", "the source changed");
    assert_eq!(page[0].note, "a note through the mirror");
    assert_eq!(outline.rows(std::slice::from_ref(&inside), None).unwrap()[0].text, "child, edited in the mirror");
}

/// How long reading a page takes on a big outline (20 000 rows), for deciding whether rows
/// should be read lazily (wiki: iOS-Cross-Platform §4.2). Run with `cargo test --release -p
/// flowie-ffi -- --ignored --nocapture page_timing`.
#[test]
#[ignore]
fn page_timing() {
    let mut outline = Outline::in_memory();
    {
        let tree = &mut outline.ws.tree;
        for i in 0..2000 {
            let top = tree.create_at(ROOT, i, &format!("Top-level bullet number {i} with some text"));
            for j in 0..9 {
                tree.create_at(top, j, &format!("A child bullet {j} under {i}, with **markup** and 中文"));
            }
        }
        outline.ws.commit_with_imports(None, None, None);
    }
    let started = std::time::Instant::now();
    let n = 20;
    let mut rows = 0;
    for _ in 0..n {
        rows = outline.page(None).unwrap().len();
    }
    let page = started.elapsed() / n;
    let id = outline.page(None).unwrap()[rows / 2].id.clone();
    let started = std::time::Instant::now();
    for i in 0..n {
        let out = outline.set_text(&id, &format!("typed {i}")).unwrap();
        outline.rows(&out.changed_ids, None).unwrap();
    }
    let typing = started.elapsed() / n;
    eprintln!("page(): {rows} rows in {page:?}; set_text + rows(): {typing:?}");
    let started = std::time::Instant::now();
    let pages = outline.index_pages().len();
    eprintln!("index_pages(): {pages} pages in {:?}", started.elapsed());
    assert_eq!(rows, 20_000);
}

#[test]
fn an_outline_on_disk_keeps_its_edits() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("outline.sqlite").to_string_lossy().into_owned();
    let mut outline = Outline::open(db.as_ref()).unwrap();
    let id = add(&mut outline, None, 0, "saved");
    outline.set_text(&id, "saved, edited").unwrap();
    outline.flush().unwrap();
    drop(outline);
    let mut outline = Outline::open(db.as_ref()).unwrap();
    assert_eq!(outline.page(None).unwrap()[0].text, "saved, edited");
}

#[test]
fn page_levels_counts_unfolded_visible_depth_up_to_three() {
    let mut outline = Outline::in_memory();
    assert_eq!(outline.page_levels(None).unwrap(), 0);
    let a = add(&mut outline, None, 0, "a");
    assert_eq!(outline.page_levels(None).unwrap(), 1);
    let b = add(&mut outline, Some(&a), 0, "b");
    assert_eq!(outline.page_levels(None).unwrap(), 2);
    // Collapsing `a` leaves only depth-0 rows on screen, yet the page still has 2 levels.
    outline.set_collapsed(&a, true).unwrap();
    assert_eq!(outline.page_levels(None).unwrap(), 2);
    let c = add(&mut outline, Some(&b), 0, "c");
    assert_eq!(outline.page_levels(None).unwrap(), 3);
    assert_eq!(outline.page_levels(Some(a.as_str())).unwrap(), 2);
    assert_eq!(outline.page_levels(Some(b.as_str())).unwrap(), 1);
    assert_eq!(outline.page_levels(Some(c.as_str())).unwrap(), 0);
    // Hiding completed rows drops levels whose only bullets are completed.
    outline.set_completed(&c, true).unwrap();
    outline.set_hide_completed(true);
    assert_eq!(outline.page_levels(None).unwrap(), 2);
}
#[test]
fn create_after_and_a_search_page() {
    let mut outline = Outline::in_memory();
    let a = add(&mut outline, None, 0, "groceries");
    let out = outline.create_after(&a, "work\nlater").unwrap();
    let b = out.focus.clone().unwrap().id;
    assert_eq!(out.focus.map(|f| (f.start, f.end)), Some((10, 10)), "at the end of its text");
    add(&mut outline, Some(&a), 0, "buy #milk");
    assert_eq!(texts(&mut outline), [("groceries".into(), 0), ("buy #milk".into(), 1), ("work later".into(), 0)]);
    let found = outline.page_matching(None, Some("#milk")).unwrap();
    assert_eq!(found.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(), ["groceries", "buy #milk"], "the match and above it");
    assert_eq!(outline.page_matching(None, Some("  ")).unwrap().len(), 3, "a blank query is none");
    assert!(outline.undo().structure_changed);
    assert!(matches!(outline.create_after("nope", "x"), Err(Error::NoSuchBullet { .. })));
    assert!(outline.bullet(&b).is_ok());
}

#[test]
fn results_serialize_as_the_browser_reads_them() {
    let run = &markup("[a](https://x.org)")[1];
    assert_eq!(serde_json::to_value(&run.kind).unwrap(), serde_json::json!({ "type": "link", "url": "https://x.org", "bullet": null }));
    assert_eq!(serde_json::to_value(RunKind::Math { display: true }).unwrap(), serde_json::json!({ "type": "math", "display": true }));
    assert_eq!(serde_json::from_str::<TextFormat>("\"strikethrough\"").unwrap(), TextFormat::Strikethrough);
    let place: DropPlace = serde_json::from_str(r#"{"parent":null,"index":2,"depth":0}"#).unwrap();
    assert_eq!(place, DropPlace { parent: None, index: 2, depth: 0 });
    let mut outline = Outline::in_memory();
    let a = add(&mut outline, None, 0, "page");
    add(&mut outline, Some(&a), 0, "child");
    let page = serde_json::to_value(&outline.index_pages()[0]).unwrap();
    assert!(page["digest"].is_string(), "u64 as a decimal string");
}
