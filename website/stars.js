// Fill in the GitHub star count on every [data-gh-stars] element, from GitHub's API. The
// count is cached for an hour (the API allows 60 unauthenticated requests an hour); without
// script, or if GitHub can't be reached, the count stays hidden.
(function () {
  var REPO = "wesleyel/flowie", KEY = "flowie-stars", HOUR = 3600 * 1000;
  function show(n) {
    if (typeof n !== "number") return;
    var text = n >= 1000 ? (n / 1000).toFixed(n >= 10000 ? 0 : 1) + "k" : String(n);
    document.querySelectorAll("[data-gh-stars]").forEach(function (el) {
      el.lastElementChild.textContent = text;
      el.hidden = false;
    });
  }
  var cached = null;
  try { cached = JSON.parse(localStorage.getItem(KEY) || "null"); } catch (e) {}
  if (cached && Date.now() - cached.at < HOUR) return show(cached.n);
  fetch("https://api.github.com/repos/" + REPO, { headers: { Accept: "application/vnd.github+json" } })
    .then(function (r) { return r.ok ? r.json() : null; })
    .then(function (repo) {
      if (!repo) return show(cached && cached.n);
      try { localStorage.setItem(KEY, JSON.stringify({ n: repo.stargazers_count, at: Date.now() })); } catch (e) {}
      show(repo.stargazers_count);
    })
    .catch(function () { show(cached && cached.n); });
})();
