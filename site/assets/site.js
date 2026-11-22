// Corral's site: theme switch (ink / paper), copy buttons, and for the docs the
// sidebar, the "On this page" list and the previous / next links, all built
// from the one list below so every page stays in step.

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

document.querySelectorAll(".theme-switch button").forEach((b) => {
  b.addEventListener("click", () => {
    document.documentElement.dataset.theme = b.dataset.theme;
    store("corral-theme", b.dataset.theme);
    paintSwitch();
  });
});
paintSwitch();

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


// ---------------------------------------------------------------- docs

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

  // On a phone the sections open over the page (see .side in the CSS): the button
  // says Close while it is open, and Esc or picking a page closes it too.
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
    addEventListener("keydown", (e) => { if (e.key === "Escape" && side.classList.contains("open")) setOpen(false); });
  }
}

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
    addEventListener("scroll", mark, { passive: true });
    mark();
  } else {
    toc.remove();
  }
}
