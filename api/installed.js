// Corral's install counter. When an installation finishes, `corral install` sends one
// anonymous ping here: the version, the system (linux or wsl), whether it is a new
// install or an update (and on an update, the version it replaced), and where it came
// from (the published package or a clone). When it stops because something required
// is missing, the ping says `stopped` and which checks failed, by the names Corral
// gives them (herdr, r, …); install.sh sends one too when it stops before Corral is
// on the machine (system macos or other, a processor that isn't x86_64, no tar) or
// when a published package doesn't check out (sha256, sha256_missing, attestation).
// `corral uninstall` sends one with kind "uninstall". The
// country comes from Vercel's geolocation of the request; the IP itself is never
// stored. CORRAL_NO_TELEMETRY=1, DO_NOT_TRACK=1 or `corral config telemetry off`
// stop the ping.
//
// Each ping is one tiny file in a private Vercel Blob store, and its name carries the
// data, so counting only lists names:
//   installs/<day>/<country>-<system>-<kind>-<source>-<version>[-<extra>]-<random>.json
// <extra> is the version an update replaced (Corral 0.1.17 and later), or a stop's
// failed checks joined by "+". Older pings have none; the random suffix never reads
// as a version (it has no dots) and stops always carry their list, so every name
// counts right.
//
// Since Corral 0.1.20 an install, update or stop also says where Corral runs, in
// categories that name no person, machine or path: the agent (claude, codex, gemini,
// opencode, none, other), the Ubuntu or Debian release (noble, bookworm…), herdr's
// version, R's major version and whether uv is there. They go in one more segment,
// e_<agent>_<release>_<herdr>_<r>_<python>, which older names never have; each value
// is checked here against a list or a pattern, so nothing else can get in. An
// uninstall says only what every ping says (kind "uninstall").
//
// Since Corral 0.1.23 an install or update also says which of the R packages it
// installs by itself didn't go in (httpgd, nanoparquet, duckdb), so the project sees
// where installs on Linux get stuck: f_<package>[+<package>…], only those three names.
//
// A ping must have exactly that shape, or nothing is stored: a POST that isn't JSON,
// is over 2 KB, lacks a field or carries a value outside its list or pattern gets a
// 400 and never reaches the Blob store. Optional fields a real client may send
// imperfectly (an update's `from`) are left out instead. Unknown fields are ignored,
// so a newer client still counts. On top of this, a Vercel Firewall rule (in the
// project's settings, not in this repository) limits how many POSTs one address can
// send here in a few minutes; one install sends one.
//
// GET /api/installed?token=STATS_TOKEN returns the totals; `updates` counts each
// "<from> -> <version>" ("unknown" when the ping didn't say), `missing` how many
// stops each check caused, agent, distro, herdr, r and python the pings that said
// where Corral runs, and r_failed how many installs each R package failed in.
import { put, list } from "@vercel/blob";
import { timingSafeEqual } from "node:crypto";

const VERSION = /^\d{1,3}\.\d{1,3}\.\d{1,3}$/;
// The checks a stop can name: corral install's required ones, then install.sh's.
const CHECKS = ["herdr", "herdr_old", "r", "r_packages", "folder", "other", "system", "arch", "basics",
  "sha256", "sha256_missing", "attestation"];
const AGENTS = ["claude", "codex", "gemini", "opencode", "none", "other"];
const ENV = ["agent", "distro", "herdr", "r", "python"];
// The R packages an install puts in by itself and can report as failed.
const R_PACKAGES = ["httpgd", "nanoparquet", "duckdb"];
const MAX_BODY = 2048;
const one = (value, allowed) => typeof value === "string" && allowed.includes(value);
const like = (value, pattern) => typeof value === "string" && pattern.test(value);

// The Blob name for a ping, or null when it isn't shaped like one a Corral client sends.
export function pingName(body, country, day) {
  if (!body || typeof body !== "object" || Array.isArray(body)) return null;
  const { kind, system, source, version } = body;
  if (!one(kind, ["new", "update", "stopped", "uninstall"])) return null;
  if (!one(system, ["linux", "wsl", "macos", "other"])) return null;
  if (!one(source, ["release", "clone"])) return null;
  // install.sh doesn't know the version when it stops before downloading Corral.
  if (!(like(version, VERSION) || (version === "unknown" && kind === "stopped"))) return null;
  let extra = "";
  if (kind === "update" && like(body.from, VERSION)) extra = `-${body.from}`;
  // Optional, like `from`: left out unless it is a short list of known names.
  if ((kind === "new" || kind === "update") && Array.isArray(body.r_failed)) {
    const failed = [...new Set(body.r_failed)];
    if (failed.length > 0 && failed.length <= R_PACKAGES.length && failed.every((p) => one(p, R_PACKAGES))) extra += `-f_${failed.join("+")}`;
  }
  if (kind === "stopped") {
    const missing = Array.isArray(body.missing) ? [...new Set(body.missing)] : [];
    if (missing.length === 0 || missing.length > CHECKS.length || !missing.every((m) => one(m, CHECKS))) return null;
    extra = `-${missing.join("+")}`;
  }
  if (kind !== "uninstall" && ENV.some((k) => body[k] !== undefined)) {
    const e = [body.agent, body.distro, body.herdr, body.r, body.python];
    const fine = one(e[0], AGENTS)
      && like(e[1], /^[a-z]{1,16}$/)
      && (one(e[2], ["none", "unknown"]) || like(e[2], /^\d{1,4}(\.\d{1,4}){1,2}$/))
      && (one(e[3], ["none", "unknown"]) || like(e[3], /^\d{1,2}\.\d{1,2}$/))
      && one(e[4], ["uv", "none"]);
    if (!fine) return null;
    extra += `-e_${e.join("_")}`;
  }
  return `installs/${day}/${country}-${system}-${kind}-${source}-${version}${extra}.json`;
}

const sameToken = (given, token) => {
  const a = Buffer.from(String(given || "")), b = Buffer.from(token);
  return a.length === b.length && timingSafeEqual(a, b);
};

export default async function handler(req, res) {
  if (req.method === "POST") {
    const type = String(req.headers["content-type"] || "");
    if (!type.startsWith("application/json") || Number(req.headers["content-length"] || 0) > MAX_BODY) {
      res.status(400).end();
      return;
    }
    let body;
    try {
      body = typeof req.body === "string" ? JSON.parse(req.body) : req.body;
    } catch (e) {
      body = null;
    }
    const country = String(req.headers["x-vercel-ip-country"] || "XX").slice(0, 2).toUpperCase().replace(/[^A-Z]/g, "X");
    const name = pingName(body, country, new Date().toISOString().slice(0, 10));
    if (!name) {
      res.status(400).end();
      return;
    }
    await put(name, "{}", {
      access: "private",
      addRandomSuffix: true,
      contentType: "application/json",
    });
    res.status(204).end();
    return;
  }

  if (req.method === "GET") {
    const token = process.env.STATS_TOKEN;
    if (!token || !sameToken(req.query.token, token)) {
      res.status(404).end();
      return;
    }
    const counts = { total: 0, country: {}, system: {}, kind: {}, source: {}, version: {}, updates: {}, missing: {},
      agent: {}, distro: {}, herdr: {}, r: {}, python: {}, r_failed: {}, day: {} };
    const add = (group, key) => { counts[group][key] = (counts[group][key] || 0) + 1; };
    let cursor;
    do {
      const page = await list({ prefix: "installs/", cursor, limit: 1000 });
      for (const blob of page.blobs) {
        const [, day, name] = blob.pathname.split("/");
        const [country, system, kind, source, version, ...rest] = name.split("-");
        const sixth = rest[0];
        const env = rest.find((r) => r.startsWith("e_"));
        if (env) env.slice(2).split("_").forEach((v, i) => ENV[i] && add(ENV[i], v));
        const failed = rest.find((r) => r.startsWith("f_"));
        if (failed) failed.slice(2).split("+").forEach((p) => add("r_failed", p));
        counts.total += 1;
        add("day", day);
        add("country", country);
        add("system", system);
        add("kind", kind);
        add("source", source);
        add("version", version);
        if (kind === "update") add("updates", `${VERSION.test(sixth || "") ? sixth : "unknown"} -> ${version}`);
        if (kind === "stopped") for (const m of (sixth || "other").split("+")) add("missing", m);
      }
      cursor = page.hasMore ? page.cursor : undefined;
    } while (cursor);
    res.setHeader("Cache-Control", "no-store");
    res.status(200).json(counts);
    return;
  }

  res.setHeader("Allow", "GET, POST");
  res.status(405).end();
}
