// The download page's version table: every published release, newest first, with a direct
// link to each package. Read from GitHub's releases API when the page opens, so a new
// release shows up without rebuilding the site. Without script, or if the API can't be
// reached, the link to GitHub Releases already in the table's place stays.
(function () {
  var REPO = "wesleyel/flowie";
  var PACKAGES = [
    { suffix: "-macos.zip", label: "zip" },
    { suffix: "-windows-x64.zip", label: "zip" },
    { suffix: "-linux-x86_64.AppImage", label: "AppImage", also: { suffix: "-linux-x86_64.tar.gz", label: "tar.gz" } }
  ];

  // A bilingual label: lang.js shows the visitor's language.
  function both(en, zh) {
    var f = document.createDocumentFragment();
    var a = document.createElement("span");
    a.lang = "en";
    a.textContent = en;
    var b = document.createElement("span");
    b.lang = "zh-CN";
    b.textContent = zh;
    f.appendChild(a);
    f.appendChild(b);
    return f;
  }

  function cell(tag, content) {
    var c = document.createElement(tag);
    if (typeof content === "string") c.textContent = content;
    else if (content) c.appendChild(content);
    return c;
  }

  function link(href, text) {
    var a = document.createElement("a");
    a.href = href;
    a.textContent = text;
    return a;
  }

  function asset(release, suffix) {
    return (release.assets || []).filter(function (a) {
      return a.name.slice(-suffix.length) === suffix && a.name.indexOf(".sha256") < 0;
    })[0];
  }

  function render(box, releases) {
    releases = releases.filter(function (r) { return !r.draft; });
    if (!releases.length) return;
    // Candidates (`vX.Y.Z-rc.N`) are published as ordinary releases too.
    function candidate(r) { return r.prerelease || /-rc\.\d+$/.test(r.tag_name); }
    var latest = releases.filter(function (r) { return !candidate(r); })[0];
    var table = document.createElement("table");
    var head = table.createTHead().insertRow();
    [both("Version", "版本"), both("Released", "发布日期"), cell("span", "macOS"), cell("span", "Windows"), cell("span", "Linux")]
      .forEach(function (label) { head.appendChild(cell("th", label)); });
    var body = table.createTBody();
    releases.forEach(function (r) {
      var row = body.insertRow();
      var version = cell("td", link(r.html_url, r.tag_name.replace(/^v/, "")));
      if (r === latest) version.appendChild(cell("span", both("Latest", "最新"))).className = "tag";
      else if (candidate(r)) version.appendChild(cell("span", both("Candidate", "候选版"))).className = "tag muted";
      row.appendChild(version);
      row.appendChild(cell("td", (r.published_at || "").slice(0, 10)));
      PACKAGES.forEach(function (p) {
        var td = cell("td");
        [p, p.also].forEach(function (q, i) {
          var a = q && asset(r, q.suffix);
          if (!a) return;
          if (i && td.firstChild) td.appendChild(document.createTextNode(" · "));
          td.appendChild(link(a.browser_download_url, q.label));
        });
        if (!td.firstChild) td.textContent = "—";
        row.appendChild(td);
      });
    });
    box.textContent = "";
    box.appendChild(table);
  }

  document.addEventListener("DOMContentLoaded", function () {
    var box = document.getElementById("versions-table");
    if (!box || !window.fetch) return;
    fetch("https://api.github.com/repos/" + REPO + "/releases?per_page=30")
      .then(function (res) { return res.ok ? res.json() : Promise.reject(res.status); })
      .then(function (releases) { render(box, releases); })
      .catch(function () {});
  });
})();
