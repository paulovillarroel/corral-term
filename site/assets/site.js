// Corral's site: theme switch (ink / paper), copy buttons, and for the docs the
// sidebar, the "On this page" list and the previous / next links, all built
// from the one list below so every page stays in step. Links inside the site
// swap the page in place (see "moving between pages" at the end).

const NAV = [
  ["Start here", [
    ["index.html", "Overview"],
    ["install.html", "Install"],
    ["quickstart.html", "Quick start"],
    ["keys.html", "Keyboard shortcuts"],
  ]],
  ["Using Corral", [
    ["layouts.html", "Layouts and panes"],
    ["editor.html", "The script editor"],
    ["consoles.html", "Consoles: R and Python"],
    ["data.html", "Variables and tables"],
    ["plots.html", "Plots and Mirador"],
    ["agents.html", "Working with agents"],
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
    side.innerHTML = head + NAV.map(([group, pages]) => {
      const items = pages.map(([href, title]) => {
        flat.push([href, title]);
        const current = href === here ? ' aria-current="page"' : "";
        return `<li><a href="${href}"${current}>${title}</a></li>`;
      }).join("");
      return `<h4>${group}</h4><ul>${items}</ul>`;
    }).join("");

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
