// Show one language at a time: the visitor's choice, else the browser's. Without this
// script both languages stay visible (style.css only hides one under html.lang-*).
(function () {
  var root = document.documentElement;
  var saved = null;
  try { saved = localStorage.getItem("flowie-lang"); } catch (e) {}
  var lang = saved === "en" || saved === "zh" ? saved
    : (/^zh/i.test(navigator.language || "") ? "zh" : "en");

  function apply(l) {
    lang = l;
    root.classList.remove("lang-en", "lang-zh");
    root.classList.add("lang-" + l);
    root.lang = l === "zh" ? "zh-CN" : "en";
    document.querySelectorAll("[data-lang-set]").forEach(function (b) {
      b.setAttribute("aria-pressed", String(b.getAttribute("data-lang-set") === l));
    });
  }
  function choose(l) {
    try { localStorage.setItem("flowie-lang", l); } catch (e) {}
    apply(l);
  }

  apply(lang);
  document.addEventListener("DOMContentLoaded", function () {
    apply(lang);
    document.querySelectorAll("[data-lang-set]").forEach(function (b) {
      b.addEventListener("click", function () { choose(b.getAttribute("data-lang-set")); });
    });
    document.querySelectorAll("[data-lang-toggle]").forEach(function (b) {
      b.addEventListener("click", function () { choose(lang === "zh" ? "en" : "zh"); });
    });
  });
})();
