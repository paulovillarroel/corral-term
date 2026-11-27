// Corral's site: theme switch (ink / paper), copy buttons, the phone note's share
// link, and for the docs the
// sidebar, the "On this page" list and the previous / next links, all built
// from the one list below so every page stays in step. Links inside the site
// swap the page in place (see "moving between pages" at the end).

const NAV = [
  ["Start here", [
    ["index.html", "Overview"],
    ["philosophy.html", "Why Corral"],
    ["install.html", "Install"],
    ["quickstart.html", "Quick start"],
    ["keys.html", "Keyboard shortcuts"],
  ]],
  ["Using Corral", [
    ["layouts.html", "Layouts and panes"],
    ["editor.html", "The script editor"],
    ["consoles.html", "Consoles: R and Python"],
    ["data.html", "Variables and tables"],
    ["sql.html", "SQL"],
    ["plots.html", "Plots and Mirador"],
  ]],
  ["Harness and security", [
    ["harness.html", "The harness"],
    ["profiles.html", "Security profiles"],
    ["agents.html", "Working with agents"],
    ["security.html", "Security model"],
  ]],
  ["Configure", [
    ["themes.html", "Themes"],
    ["config.html", "Configuration"],
  ]],
  ["Reference", [
    ["architecture.html", "Architecture"],
    ["commands.html", "Commands"],
    ["performance.html", "Performance"],
  ]],
  ["Help", [
    ["troubleshooting.html", "Troubleshooting and limits"],
  ]],
];

// ---------------------------------------------------------------- docs search

// The docs pages, read once (the first time the search box is used) and cut into
// sections: no index to keep in step with the pages, and nothing loaded until then.
// It stays across the in-place page swaps below.
let docIndex = null;
const slug = (t) => t.toLowerCase().trim().replace(/[^a-z0-9]+/g, "-").replace(/(^-|-$)/g, "");
const escapeHtml = (t) => t.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);

function loadDocIndex() {
  if (docIndex) return docIndex;
  const base = new URL(location.pathname.includes("/docs/") ? "./" : "docs/", location.href);
  docIndex = Promise.all(NAV.flatMap(([, pages]) => pages).map(async ([href, title]) => {
    try {
      const res = await fetch(new URL(href, base));
      const doc = new DOMParser().parseFromString(await res.text(), "text/html");
      const main = doc.querySelector(".content");
      if (!main) return [];
      const out = [];
      let cur = { href, title, section: title, id: "", text: "" };
      for (const el of main.children) {
        if (el.tagName === "H2" || el.tagName === "H3") {
          out.push(cur);
          cur = { href, title, section: el.textContent.trim(), id: el.id || slug(el.textContent), text: "" };
        } else if (el.tagName !== "H1" && !el.classList.contains("pager")) {
          // Table cells and list items run together in textContent: a space between them.
          el.querySelectorAll("td, th, li, dt, dd, p, pre, br").forEach((n) => n.append(" "));
          cur.text += " " + el.textContent;
        }
      }
      out.push(cur);
      return out.map((e) => ({ ...e, text: e.text.replace(/\s+/g, " ").trim(), low: (e.title + " " + e.section + " " + e.text).toLowerCase() }));
    } catch (e) {
      return [];
    }
  })).then((all) => all.flat());
  return docIndex;
}

// Sections holding every word, best first: in the section's title, then the page's, then the text.
function searchDocs(index, query) {
  const words = query.toLowerCase().split(/\s+/).filter((w) => w.length > 1);
  if (!words.length) return [];
  return index
    .filter((e) => words.every((w) => e.low.includes(w)))
    .map((e) => {
      const sec = e.section.toLowerCase(), page = e.title.toLowerCase(), text = e.text.toLowerCase();
      const score = words.reduce((s, w) => s + (sec.includes(w) ? 6 : 0) + (page.includes(w) ? 3 : 0) + Math.min(3, text.split(w).length - 1), 0);
      return { e, score };
    })
    .sort((a, b) => b.score - a.score)
    .slice(0, 8)
    .map(({ e }) => e);
}

function snippet(text, words) {
  const low = text.toLowerCase();
  const at = Math.max(0, words.map((w) => low.indexOf(w)).filter((i) => i >= 0).sort((a, b) => a - b)[0] ?? 0);
  let start = Math.max(0, at - 50);
  if (start) start = text.indexOf(" ", start) + 1 || start; // from a whole word
  let cut = (start ? "…" : "") + text.slice(start, start + 150) + (start + 150 < text.length ? "…" : "");
  cut = escapeHtml(cut);
  for (const w of words) cut = cut.replace(new RegExp(w.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"), "gi"), (m) => `<mark>${m}</mark>`);
  return cut;
}

function setupDocSearch(box, signal) {
  const input = box.querySelector("input");
  const hits = box.querySelector(".doc-hits");
  let active = -1, timer = 0;
  const links = () => [...hits.querySelectorAll("a")];
  const mark = (i) => {
    const all = links();
    active = all.length ? (i + all.length) % all.length : -1;
    all.forEach((a, k) => a.classList.toggle("on", k === active));
    if (active >= 0) all[active].scrollIntoView({ block: "nearest" });
  };
  const close = () => { hits.hidden = true; active = -1; };
  // The menu scrolls, and would cut off a panel inside it: the results float, fixed,
  // under the box, as wide as the screen allows.
  const place = () => {
    const r = input.getBoundingClientRect();
    hits.style.left = r.left + "px";
    hits.style.top = r.bottom + 4 + "px";
    hits.style.width = Math.min(480, innerWidth - r.left - 16) + "px";
  };
  addEventListener("resize", () => { if (!hits.hidden) place(); }, { signal });
  box.closest(".side")?.addEventListener("scroll", close, { signal });
  const run = async () => {
    const q = input.value.trim();
    if (q.length < 2) return close();
    const index = await loadDocIndex();
    if (input.value.trim() !== q) return; // typed on meanwhile
    const words = q.toLowerCase().split(/\s+/).filter((w) => w.length > 1);
    const found = searchDocs(index, q);
    hits.innerHTML = found.length
      ? found.map((e) => `<li><a href="${e.href}${e.id ? "#" + e.id : ""}"><b>${escapeHtml(e.title)}${e.section !== e.title ? " › " + escapeHtml(e.section) : ""}</b><span>${snippet(e.text, words)}</span></a></li>`).join("")
      : `<li class="none">Nothing found for “${escapeHtml(q)}”.</li>`;
    hits.hidden = false;
    place();
    mark(0);
  };
  input.addEventListener("focus", () => { loadDocIndex(); if (input.value.trim().length >= 2) run(); }, { signal });
  input.addEventListener("input", () => { clearTimeout(timer); timer = setTimeout(run, 60); }, { signal });
  input.addEventListener("keydown", (e) => {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") { e.preventDefault(); mark(active + (e.key === "ArrowDown" ? 1 : -1)); }
    else if (e.key === "Enter" && active >= 0) { e.preventDefault(); links()[active].click(); close(); }
    else if (e.key === "Escape") { close(); input.blur(); }
  }, { signal });
  hits.addEventListener("click", (e) => { if (e.target.closest("a")) close(); }, { signal });
  document.addEventListener("click", (e) => { if (!box.contains(e.target)) close(); }, { signal });
  // "/" anywhere but in a field: to the search box (the menu opens first on a phone).
  addEventListener("keydown", (e) => {
    if (e.key !== "/" || e.metaKey || e.ctrlKey || e.altKey || /^(INPUT|TEXTAREA|SELECT)$/.test(document.activeElement?.tagName || "")) return;
    e.preventDefault();
    const toggle = document.querySelector(".menu-toggle");
    if (toggle && getComputedStyle(toggle).display !== "none" && !document.querySelector(".side.open")) toggle.click();
    input.focus();
  }, { signal });
}

function store(key, value) {
  try {
    if (value === undefined) return localStorage.getItem(key);
    localStorage.setItem(key, value);
  } catch (e) {
    return null;
  }
}

// ---------------------------------------------------------------- theme

function currentTheme() {
  const set = document.documentElement.dataset.theme;
  if (set) return set;
  return "ink"; // dark by default, like a terminal; paper only when chosen
}

function paintSwitch() {
  const theme = currentTheme();
  document.querySelectorAll(".theme-switch button").forEach((b) => {
    b.setAttribute("aria-pressed", String(b.dataset.theme === theme));
  });
  document.querySelectorAll("picture[data-themed] source").forEach((s) => {
    // Charts follow the site's theme, not only the system's.
    s.media = theme === "ink" ? "all" : "not all";
  });
}

// ---------------------------------------------------------------- copy

function addCopy(button, text) {
  button.addEventListener("click", async () => {
    try {
      await navigator.clipboard.writeText(text());
      const old = button.textContent;
      button.textContent = "Copied";
      setTimeout(() => (button.textContent = old), 1400);
    } catch (e) {
      button.textContent = "Select and copy";
    }
  });
}

// ---------------------------------------------------------------- each page

// Everything that belongs to the page in view. It runs on load and again after an
// in-place navigation; listeners on window go away with the page they belong to.
let pageListeners = null;

function init() {
  if (pageListeners) pageListeners.abort();
  pageListeners = new AbortController();
  const signal = pageListeners.signal;

  document.querySelectorAll(".theme-switch button").forEach((b) => {
    b.addEventListener("click", () => {
      document.documentElement.dataset.theme = b.dataset.theme;
      store("corral-theme", b.dataset.theme);
      paintSwitch();
    });
  });
  paintSwitch();

  document.querySelectorAll(".install").forEach((box) => {
    const b = box.querySelector(".copy");
    if (b) addCopy(b, () => box.querySelector("code").textContent.trim());
  });

  // The phone note's link opens the share sheet where there is one; elsewhere it stays
  // an email to send the link on. If the sheet ends without sharing (closed, failed, or
  // with nowhere to send it: browsers report these alike), the link becomes the email,
  // for the reader to tap if they want it.
  document.querySelectorAll("a.share").forEach((a) => {
    if (!navigator.share) return;
    let email = false;
    a.addEventListener("click", (e) => {
      if (email) return; // the email link, as without a share sheet
      e.preventDefault();
      navigator.share({ title: "Corral", text: "Corral: R, Python and SQL in the terminal, with your AI agent. It installs on Linux or Windows (WSL).", url: "https://corral-term.dev/" })
        .catch(() => { email = true; a.textContent = "Email yourself the link"; });
    }, { signal });
  });

  document.querySelectorAll(".content pre").forEach((pre) => {
    if (!pre.querySelector("code")) return; // screen captures, not commands
    const b = document.createElement("button");
    b.className = "copy";
    b.type = "button";
    b.textContent = "Copy";
    // Prompts ($ ) are decoration: leave them out of what gets copied.
    addCopy(b, () => {
      const clone = pre.querySelector("code").cloneNode(true);
      clone.querySelectorAll(".p").forEach((p) => p.remove());
      return clone.textContent.replace(/\n$/, "");
    });
    pre.appendChild(b);
  });

  // ---- docs: the sidebar and the previous / next links

  const side = document.querySelector(".side");
  if (side) {
    const here = location.pathname.split("/").pop() || "index.html";
    const flat = [];
    const head = '<div class="side-head"><b>Documentation</b><button type="button" class="side-close">Close</button></div>';
    const search = '<div class="doc-search" role="search"><input type="search" placeholder="Search the docs" aria-label="Search the docs" autocomplete="off" spellcheck="false"><kbd>/</kbd><ol class="doc-hits" hidden></ol></div>';
    side.innerHTML = head + search + NAV.map(([group, pages]) => {
      const items = pages.map(([href, title]) => {
        flat.push([href, title]);
        const current = href === here ? ' aria-current="page"' : "";
        return `<li><a href="${href}"${current}>${title}</a></li>`;
      }).join("");
      return `<h4>${group}</h4><ul>${items}</ul>`;
    }).join("");

    setupDocSearch(side.querySelector(".doc-search"), signal);

    const i = flat.findIndex(([href]) => href === here);
    const pager = document.querySelector(".pager");
    if (pager && i >= 0) {
      const prev = flat[i - 1];
      const next = flat[i + 1];
      pager.innerHTML =
        (prev ? `<a class="prev" href="${prev[0]}"><span>Previous</span><b>${prev[1]}</b></a>` : "") +
        (next ? `<a class="next" href="${next[0]}"><span>Next</span><b>${next[1]}</b></a>` : "");
    }

    // On a phone the sections open over the whole screen (see .side in the CSS);
    // Close, Esc or picking a page closes them.
    const toggle = document.querySelector(".menu-toggle");
    if (toggle) {
      toggle.setAttribute("aria-expanded", "false");
      const setOpen = (open) => {
        side.classList.toggle("open", open);
        document.body.classList.toggle("menu-open", open);
        toggle.setAttribute("aria-expanded", String(open));
      };
      toggle.addEventListener("click", () => setOpen(!side.classList.contains("open")));
      side.addEventListener("click", (e) => { if (e.target.closest("a, .side-close")) setOpen(false); });
      addEventListener("keydown", (e) => { if (e.key === "Escape" && side.classList.contains("open")) setOpen(false); }, { signal });
    }
  }

  // ---- docs: "On this page"

  const toc = document.querySelector(".toc");
  const content = document.querySelector(".content");
  if (toc && content) {
    const heads = [...content.querySelectorAll("h2, h3")];
    heads.forEach((h) => {
      if (!h.id) h.id = h.textContent.toLowerCase().trim().replace(/[^a-z0-9]+/g, "-").replace(/(^-|-$)/g, "");
    });
    if (heads.length) {
      toc.innerHTML = "<h5>On this page</h5>" + heads.map((h) =>
        `<a href="#${h.id}" class="${h.tagName === "H3" ? "sub" : ""}">${h.textContent}</a>`).join("");
      const links = [...toc.querySelectorAll("a")];
      const mark = () => {
        let on = 0;
        heads.forEach((h, k) => { if (h.getBoundingClientRect().top < 120) on = k; });
        links.forEach((a, k) => a.classList.toggle("on", k === on));
      };
      addEventListener("scroll", mark, { passive: true, signal });
      mark();
    } else {
      toc.remove();
    }
  }
}

init();

// ---------------------------------------------------------------- moving between pages

// Links inside the site swap the page in place instead of loading a new one: no blank
// screen between pages (Firefox on phones shows one), and history and the back button
// work as usual. Anything else (another site, a file, a modifier key, a new tab) is a
// normal link. Vercel's analytics counts these navigations through history.
function sitePage(a) {
  if (!a || a.target || a.hasAttribute("download")) return null;
  const url = new URL(a.href, location.href);
  if (url.origin !== location.origin || !/(\/|\.html)$/.test(url.pathname)) return null;
  if (url.pathname === location.pathname && url.hash) return null; // a section of this page
  return url;
}

async function go(url, push) {
  let doc;
  try {
    const res = await fetch(url.href);
    if (!res.ok || !(res.headers.get("content-type") || "").includes("text/html")) throw new Error(res.status);
    doc = new DOMParser().parseFromString(await res.text(), "text/html");
  } catch (e) {
    location.href = url.href; // a normal load, then
    return;
  }
  const swap = () => {
    if (push) history.pushState({}, "", url.href); // first, so relative links resolve from the new page
    document.title = doc.title;
    const desc = doc.querySelector('meta[name="description"]');
    const mine = document.querySelector('meta[name="description"]');
    if (desc && mine) mine.content = desc.content;
    document.body.className = doc.body.className;
    document.body.innerHTML = doc.body.innerHTML;
    init();
    const target = url.hash && document.getElementById(decodeURIComponent(url.hash.slice(1)));
    if (target) target.scrollIntoView();
    else scrollTo(0, 0);
  };
  if (document.startViewTransition) document.startViewTransition(swap);
  else swap();
}

document.addEventListener("click", (e) => {
  if (e.defaultPrevented || e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
  const url = sitePage(e.target.closest("a[href]"));
  if (!url) return;
  e.preventDefault();
  go(url, true);
});
addEventListener("popstate", () => go(new URL(location.href), false));
